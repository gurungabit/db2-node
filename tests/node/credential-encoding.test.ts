import test from 'node:test';
import assert from 'node:assert/strict';
import net from 'node:net';
import { Client, getConfig, type ConnectionConfig } from './helpers';

// tools/test-credential-encoding.py provisions and removes isolated Docker users.
const fixtures: Array<{ label: string; user: string; password: string }> =
  JSON.parse(process.env.DB2_TEST_CREDENTIAL_FIXTURES || '[]');
const sslPort = Number(process.env.DB2_TEST_SSL_PORT);
const caCert = process.env.DB2_TEST_CA_CERT;

function ddms(bytes: Buffer): Buffer[] {
  const objects: Buffer[] = [];
  for (let offset = 0; offset < bytes.length;) {
    assert.ok(offset + 6 <= bytes.length, 'Complete DSS header');
    const length = bytes.readUInt16BE(offset);
    assert.ok(length >= 10 && offset + length <= bytes.length, 'Complete handshake DSS');
    objects.push(bytes.subarray(offset + 6, offset + length));
    offset += length;
  }
  return objects;
}

function parameter(object: Buffer, codePoint: number): Buffer {
  for (let offset = 4; offset < object.readUInt16BE(0);) {
    const length = object.readUInt16BE(offset);
    assert.ok(length >= 4 && offset + length <= object.length);
    if (object.readUInt16BE(offset + 2) === codePoint) return object.subarray(offset + 4, offset + length);
    offset += length;
  }
  assert.fail(`Missing DDM parameter 0x${codePoint.toString(16)}`);
}

async function traceSession(run: (port: number) => Promise<void>) {
  const sent: Buffer[] = [];
  const received: Buffer[] = [];
  const sockets = new Set<net.Socket>();
  const config = getConfig();
  const proxy = net.createServer((client) => {
    const server = net.connect(config.port ?? 50000, config.host);
    sockets.add(client); sockets.add(server);
    client.on('data', (bytes) => sent.push(Buffer.from(bytes)));
    server.on('data', (bytes) => received.push(Buffer.from(bytes)));
    client.pipe(server).pipe(client);
    client.on('error', () => server.destroy());
    server.on('error', () => client.destroy());
    client.on('close', () => { server.destroy(); sockets.delete(client); sockets.delete(server); });
  });
  await new Promise<void>((resolve) => proxy.listen(0, '127.0.0.1', resolve));
  try {
    await run((proxy.address() as net.AddressInfo).port);
    return { sent: ddms(Buffer.concat(sent)), received: ddms(Buffer.concat(received)) };
  } finally {
    for (const socket of sockets) socket.destroy();
    await new Promise<void>((resolve) => proxy.close(() => resolve()));
  }
}

test('LUW wire reply disables Unicode and SECCHK uses CCSID 500', {
  skip: fixtures.length === 0 && 'No disposable credential fixtures supplied',
}, async () => {
  const fixture = fixtures.find(({ label }) => label === 'reported punctuation');
  assert.ok(fixture);
  const wire = await traceSession(async (port) => {
    const client = new Client({ ...getConfig(), ...fixture, host: '127.0.0.1', port });
    try { await client.connect(); }
    finally { await client.close(); }
  });
  const attributes = wire.received.find((ddm) => ddm.readUInt16BE(2) === 0x1443);
  assert.ok(attributes, 'EXSATRD received');
  const managers = parameter(attributes, 0x1404);
  let unicodeLevel: number | undefined;
  for (let offset = 0; offset < managers.length; offset += 4) {
    if (managers.readUInt16BE(offset) === 0x1c08) unicodeLevel = managers.readUInt16BE(offset + 2);
  }
  assert.equal(unicodeLevel, 0, 'Fixture LUW server did not negotiate UTF-8 credentials');
  const securityCheck = wire.sent.find((ddm) => ddm.readUInt16BE(2) === 0x106e);
  assert.ok(securityCheck, 'SECCHK sent');
  // Successful IBM CLI traces on both Db2 11.5 and 12.1 send these exact
  // credential bytes, including the LUW ASCII-compatible '|' at 0x6A.
  assert.deepEqual(parameter(securityCheck, 0x11a1), Buffer.from('c18283f1f2f34f5f4a5a6aa7', 'hex'));
});

test('both encrypted mechanisms refuse a TCP downgrade before SECCHK', {
  skip: fixtures.length === 0 && 'No disposable credential fixtures supplied',
}, async () => {
  for (const securityMechanism of ['encrypted', 'encryptedPassword']) {
    for (const encryptionAlgorithm of ['aes', 'des']) {
      const wire = await traceSession(async (port) => {
        const client = new Client({ ...getConfig(), ...fixtures[0], host: '127.0.0.1', port, securityMechanism, encryptionAlgorithm });
        try { await assert.rejects(client.connect(), /refusing to send credentials without encryption/); }
        finally { await client.close(); }
      });
      assert.equal(wire.sent.some((ddm) => ddm.readUInt16BE(2) === 0x106e), false, `${securityMechanism} / ${encryptionAlgorithm}`);
    }
  }
});

test('negotiated credentials preserve password punctuation', {
  skip: fixtures.length === 0 && 'No disposable credential fixtures supplied',
  timeout: 120_000,
}, async (t) => {
  const modes: Array<[string, Partial<ConnectionConfig>]> = [
    ['TCP / SECMEC 3 / auto', {}],
    ['TCP / SECMEC 3 / explicit CCSID 500', { credentialEncoding: 'ebcdic500' }],
  ];
  if (sslPort) {
    assert.ok(caCert, 'TLS regression requires a trusted public CA');
    const tls = { port: sslPort, ssl: true, caCert, rejectUnauthorized: true };
    modes.push(
      ['verified TLS / SECMEC 3', tls],
      ['verified TLS / default mechanism', { ...tls, securityMechanism: undefined }],
      ['verified TLS / SECMEC 7 request', { ...tls, securityMechanism: 'encryptedPassword' }],
    );
  }
  for (const [mode, options] of modes) {
    for (const fixture of fixtures) {
      await t.test(`${mode} / ${fixture.label}`, async () => {
        const client = new Client({ ...getConfig(), ...fixture, ...options });
        try {
          await client.connect();
          assert.equal((await client.query('VALUES 1')).rows[0]['1'], 1);
        } finally {
          await client.close();
        }
      });
    }
  }
});

test('unnegotiated UTF-8 and explicit CCSID 037 remain explicit overrides', {
  skip: fixtures.length === 0 && 'No disposable credential fixtures supplied',
}, async () => {
  const fixture = fixtures.find(({ label }) => label === 'caret');
  assert.ok(fixture, 'Caret fixture is required');
  for (const credentialEncoding of ['utf8', 'ebcdic', 'ebcdic037']) {
    const client = new Client({ ...getConfig(), ...fixture, credentialEncoding });
    try {
      await assert.rejects(client.connect(), /check_code=0x0F.*credential_encoding=(?:Utf8|Ebcdic037)/);
    } finally {
      await client.close();
    }
  }
});

test('TLS credential regression keeps certificate and hostname verification', {
  skip: (fixtures.length === 0 || !sslPort) && 'No verified TLS fixture supplied',
}, async () => {
  assert.ok(caCert);
  for (const options of [{ caCert: undefined }, { caCert, host: '0.0.0.0' }]) {
    const client = new Client({ ...getConfig(), ...fixtures[0], port: sslPort, ssl: true, rejectUnauthorized: true, ...options });
    try { await assert.rejects(client.connect(), /certificate|issuer|name/i); }
    finally { await client.close(); }
  }
});
