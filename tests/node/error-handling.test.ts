/**
 * Error handling tests.
 * Ref: ibm_db test-query-select.js (error paths)
 */
import { describe, it, before, after } from 'node:test';
import assert from 'node:assert/strict';
import { Client } from '../../crates/db2-napi';

const cfg = () => ({
  host: process.env.DB2_TEST_HOST || 'localhost',
  port: Number(process.env.DB2_TEST_PORT) || 50000,
  database: process.env.DB2_TEST_DATABASE || 'testdb',
  user: process.env.DB2_TEST_USER || 'db2inst1',
  password: process.env.DB2_TEST_PASSWORD || 'db2wire_test_pw',
});

describe('Error handling: SQL errors', () => {
  let client: InstanceType<typeof Client>;

  before(async () => {
    client = new Client(cfg());
    await client.connect();
  });
  after(async () => { await client.close(); });

  it('rejects SQL syntax errors', async () => {
    await assert.rejects(
      () => client.query('SELCT * FORM nonexistent'),
      (err: any) => {
        assert.ok(err.message.length > 0);
        return true;
      },
    );
  });

  it('rejects reference to non-existent table', async () => {
    await assert.rejects(
      () => client.query('SELECT * FROM no_such_table_xyz_123'),
      (err: any) => {
        assert.ok(err.message.length > 0);
        return true;
      },
    );
  });

  it('rejects division by zero', async () => {
    await assert.rejects(
      () => client.query('SELECT 1/0 AS X FROM SYSIBM.SYSDUMMY1'),
      (err: any) => {
        assert.ok(err.message.length > 0);
        return true;
      },
    );
  });

  it('continues working after an error', async () => {
    // Cause an error
    await client.query('SELCT BROKEN').catch(() => {});
    // Connection should still be usable
    const r = await client.query('SELECT 1 AS V FROM SYSIBM.SYSDUMMY1');
    assert.equal(r.rows.length, 1);
  });
});

describe('Error handling: connection errors', () => {
  it('rejects query on unconnected client', async () => {
    const c = new Client(cfg());
    // Don't call connect()
    await assert.rejects(
      () => c.query('SELECT 1 FROM SYSIBM.SYSDUMMY1'),
      (err: any) => {
        assert.ok(err.message.toLowerCase().includes('not connected') ||
                   err.message.length > 0);
        return true;
      },
    );
  });
});
