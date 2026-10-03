import test from 'node:test';
import assert from 'node:assert/strict';
import net from 'node:net';
import { Client, getConfig } from './helpers';

const config = getConfig();
const encryptedServer = process.env.DB2_TEST_ENCRYPTED_AUTH === '1';

test('encrypted authentication refuses plaintext downgrade before SECCHK', { skip: encryptedServer }, async () => {
  const sent: Buffer[] = [];
  const sockets = new Set<net.Socket>();
  const proxy = net.createServer((client) => {
    const server = net.connect(config.port ?? 50000, config.host);
    sockets.add(client); sockets.add(server);
    client.on('data', (bytes) => sent.push(Buffer.from(bytes)));
    client.pipe(server).pipe(client);
    client.on('error', () => server.destroy());
    server.on('error', () => client.destroy());
    client.on('close', () => { server.destroy(); sockets.delete(client); sockets.delete(server); });
  });
  await new Promise<void>((resolve) => proxy.listen(0, '127.0.0.1', resolve));
  try {
    for (const securityMechanism of ['encrypted', 'encryptedPassword']) {
      const client = new Client({ ...config, host: '127.0.0.1', port: (proxy.address() as net.AddressInfo).port, securityMechanism });
      await assert.rejects(client.connect(), /refusing to send credentials without encryption/);
    }
    // Parse each client DSS and ensure that no Security Check with credentials was sent.
    const bytes = Buffer.concat(sent);
    for (let offset = 0; offset + 6 <= bytes.length;) {
      const length = bytes.readUInt16BE(offset);
      assert.ok(length >= 10 && offset + length <= bytes.length);
      assert.notEqual(bytes.readUInt16BE(offset + 8), 0x106e, 'SECCHK must not be sent after a downgrade');
      offset += length;
    }
    assert.equal(bytes.includes(Buffer.from(config.password)), false);
  } finally {
    for (const socket of sockets) socket.destroy();
    await new Promise<void>((resolve) => proxy.close(() => resolve()));
  }
});

test('SERVER_ENCRYPT authentication repeatedly connects and queries', { skip: !encryptedServer, timeout: 120_000 }, async (t) => {
  const algorithms = process.env.DB2_TEST_AES_ONLY === '1' ? ['aes'] : ['aes', 'des'];
  for (const encryptionAlgorithm of algorithms) {
    for (const securityMechanism of ['encrypted', 'encryptedPassword']) {
      await t.test(`${securityMechanism} / ${encryptionAlgorithm}: ten fresh sessions`, async () => {
        for (let attempt = 0; attempt < 10; attempt++) {
          const client = new Client({ ...config, securityMechanism, encryptionAlgorithm });
          await client.connect();
          try { assert.equal((await client.query('VALUES 1')).rows[0]['1'], 1); }
          finally { await client.close(); }
        }
      });
    }
  }
});


test('AES_ONLY rejects DES with an explicit negotiation error', {
  skip: !encryptedServer || process.env.DB2_TEST_AES_ONLY !== '1',
}, async () => {
  for (const securityMechanism of ['encrypted', 'encryptedPassword']) {
    const client = new Client({ ...config, securityMechanism, encryptionAlgorithm: 'des' });
    await assert.rejects(client.connect(), /encryptionAlgorithm.*(?:SECCHKCD|SQLCODE=-30082)/);
  }
});
