/**
 * TLS/SSL connection tests.
 * Requires DB2 with SSL enabled and DB2_TEST_SSL_PORT env var set.
 *
 * To run locally:
 *   cd docker/tls && bash generate-certs.sh && bash setup-db2-ssl.sh
 *   DB2_TEST_SSL_PORT=50001 npm test
 */
import { describe, it, before } from 'node:test';
import assert from 'node:assert/strict';
import { Client } from '../../crates/db2-napi';
import { resolve } from 'node:path';

const sslPort = process.env.DB2_TEST_SSL_PORT
  ? Number(process.env.DB2_TEST_SSL_PORT)
  : null;

const baseCfg = () => ({
  host: process.env.DB2_TEST_HOST || 'localhost',
  port: sslPort!,
  database: process.env.DB2_TEST_DATABASE || 'testdb',
  user: process.env.DB2_TEST_USER || 'db2inst1',
  password: process.env.DB2_TEST_PASSWORD || 'db2wire_test_pw',
  ssl: true,
});

const caCertPath = resolve(__dirname, '../../docker/tls/ca.pem');

describe('TLS: basic connection', { skip: !sslPort && 'DB2_TEST_SSL_PORT not set' }, () => {
  it('connects with rejectUnauthorized=false', async () => {
    const c = new Client({ ...baseCfg(), rejectUnauthorized: false });
    await c.connect();
    const r = await c.query('VALUES 1');
    assert.equal(r.rowCount, 1);
    await c.close();
  });

  it('connects with custom CA certificate', async () => {
    const c = new Client({
      ...baseCfg(),
      rejectUnauthorized: true,
      caCert: caCertPath,
    });
    await c.connect();
    const r = await c.query("VALUES 'tls-verified'");
    assert.equal(r.rowCount, 1);
    await c.close();
  });

  it('returns real server info over TLS', async () => {
    const c = new Client({ ...baseCfg(), rejectUnauthorized: false });
    await c.connect();
    const info = await c.serverInfo();
    assert.ok(info.productName.length > 0, 'product name should be populated');
    await c.close();
  });
});

describe('TLS: prepared statements', { skip: !sslPort && 'DB2_TEST_SSL_PORT not set' }, () => {
  it('prepares and executes over TLS', async () => {
    const c = new Client({ ...baseCfg(), rejectUnauthorized: false });
    await c.connect();

    const stmt = await c.prepare('VALUES CAST(? AS INTEGER) + 10');
    const r = await stmt.execute([5]);
    assert.equal(r.rows[0]['1'], '15');
    await stmt.close();
    await c.close();
  });
});

describe('TLS: error cases', { skip: !sslPort && 'DB2_TEST_SSL_PORT not set' }, () => {
  it('fails TLS handshake to non-SSL port', { timeout: 10000 }, async () => {
    const c = new Client({
      ...baseCfg(),
      port: Number(process.env.DB2_TEST_PORT) || 50000, // plain TCP port
      rejectUnauthorized: false,
      connectTimeout: 3000, // connect_timeout now covers TCP + TLS handshake
    });
    await assert.rejects(() => c.connect(), (err: any) => {
      assert.ok(err.message.length > 0, 'should get a timeout or TLS error');
      return true;
    });
  });

  it('fails with rejectUnauthorized=true and no custom CA', async () => {
    const c = new Client({
      ...baseCfg(),
      rejectUnauthorized: true,
      // No caCert — self-signed cert won't be in system store
    });
    await assert.rejects(() => c.connect(), (err: any) => {
      assert.ok(err.message.length > 0);
      return true;
    });
  });
});
