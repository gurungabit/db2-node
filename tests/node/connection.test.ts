/**
 * Connection lifecycle tests.
 * Ref: ibm_db test-open-close.js, test-multi-open-close.js
 */
import { describe, it } from 'node:test';
import assert from 'node:assert/strict';
import { Client } from '../../crates/db2-napi';

const cfg = () => ({
  host: process.env.DB2_TEST_HOST || 'localhost',
  port: Number(process.env.DB2_TEST_PORT) || 50000,
  database: process.env.DB2_TEST_DATABASE || 'testdb',
  user: process.env.DB2_TEST_USER || 'db2inst1',
  password: process.env.DB2_TEST_PASSWORD || 'db2wire_test_pw',
});

describe('Connection: open and close', () => {
  it('connects, runs a trivial query, and disconnects', async () => {
    const c = new Client(cfg());
    await c.connect();
    const r = await c.query('SELECT 1 AS V FROM SYSIBM.SYSDUMMY1');
    assert.equal(r.rows.length, 1);
    await c.close();
  });

  it('connects and disconnects 5 times sequentially', async () => {
    for (let i = 0; i < 5; i++) {
      const c = new Client(cfg());
      await c.connect();
      const r = await c.query('SELECT 1 AS V FROM SYSIBM.SYSDUMMY1');
      assert.equal(r.rows.length, 1);
      await c.close();
    }
  });
});

describe('Connection: bad credentials', () => {
  it('rejects wrong password', async () => {
    const c = new Client({ ...cfg(), password: 'definitely_wrong_pwd' });
    await assert.rejects(() => c.connect(), (err: any) => {
      assert.ok(err.message.length > 0);
      return true;
    });
  });

  it('rejects unreachable host (timeout)', async () => {
    const c = new Client({ ...cfg(), host: '192.0.2.1', connectTimeout: 2000 });
    await assert.rejects(() => c.connect(), (err: any) => {
      assert.ok(err.message.length > 0);
      return true;
    });
  });
});

describe('Connection: server info', () => {
  it('returns server info after connect', async () => {
    const c = new Client(cfg());
    await c.connect();
    const info = await c.serverInfo();
    assert.ok(info, 'serverInfo should exist');
    assert.ok(info.productName.length > 0, 'product name should be populated');
    await c.close();
  });
});
