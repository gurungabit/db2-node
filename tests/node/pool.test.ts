/**
 * Connection pool tests.
 * Ref: ibm_db test-pool-open.js, test-pool-query.js
 */
import { describe, it, after, afterEach } from 'node:test';
import assert from 'node:assert/strict';
import { Pool } from '../../crates/db2-napi';

const poolCfg = () => ({
  host: process.env.DB2_TEST_HOST || 'localhost',
  port: Number(process.env.DB2_TEST_PORT) || 50000,
  database: process.env.DB2_TEST_DATABASE || 'testdb',
  user: process.env.DB2_TEST_USER || 'db2inst1',
  password: process.env.DB2_TEST_PASSWORD || 'db2wire_test_pw',
  maxConnections: 5,
});

describe('Pool: basic', () => {
  let pool: InstanceType<typeof Pool>;

  after(async () => { if (pool) await pool.close(); });

  it('creates a pool and runs a query', async () => {
    pool = new Pool(poolCfg());
    const r = await pool.query('SELECT 1 AS V FROM SYSIBM.SYSDUMMY1');
    assert.equal(r.rows.length, 1);
  });
});

describe('Pool: concurrent queries', () => {
  let pool: InstanceType<typeof Pool>;

  after(async () => { if (pool) await pool.close(); });

  it('handles 10 concurrent queries', async () => {
    pool = new Pool(poolCfg());
    const promises = Array.from({ length: 10 }, (_, i) =>
      pool.query('SELECT 1 AS V FROM SYSIBM.SYSDUMMY1'),
    );
    const results = await Promise.all(promises);
    assert.equal(results.length, 10);
    for (const r of results) {
      assert.equal(r.rows.length, 1);
    }
  });
});

describe('Pool: acquire and release', () => {
  let pool: InstanceType<typeof Pool>;

  after(async () => { if (pool) await pool.close(); });
  afterEach(async () => {
    if (pool) {
      await pool.close();
      // @ts-expect-error reset between tests
      pool = undefined;
    }
  });

  it('acquires a client, uses it, and releases', async () => {
    pool = new Pool(poolCfg());
    const client = await pool.acquire();
    const r = await client.query('SELECT 1 AS V FROM SYSIBM.SYSDUMMY1');
    assert.equal(r.rows.length, 1);
    await pool.release(client);
  });

  it('releases a slot when a checked-out client is closed', async () => {
    pool = new Pool({ ...poolCfg(), maxConnections: 1 });
    const client = await pool.acquire();
    await client.close();

    const r = await pool.query('SELECT 1 AS V FROM SYSIBM.SYSDUMMY1');
    assert.equal(r.rows.length, 1);
  });
});
