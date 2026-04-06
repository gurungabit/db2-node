/**
 * Transaction tests.
 * Ref: ibm_db test-transaction.js
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

describe('Transaction: commit', () => {
  let client: InstanceType<typeof Client>;
  const table = `tmp_txc_${Date.now() % 1_000_000}`;

  before(async () => {
    client = new Client(cfg());
    await client.connect();
    await client.query(`CREATE TABLE ${table} (id INT, val INT)`);
  });
  after(async () => {
    await client.query(`DROP TABLE ${table}`).catch(() => {});
    await client.close();
  });

  it('committed data is visible', async () => {
    const tx = await client.beginTransaction();
    await tx.query(`INSERT INTO ${table} VALUES (1, 100)`);
    await tx.query(`INSERT INTO ${table} VALUES (2, 200)`);
    await tx.commit();

    const r = await client.query(`SELECT COUNT(*) AS CNT FROM ${table}`);
    assert.equal(Number(r.rows[0].CNT), 2);
  });
});

describe('Transaction: rollback', () => {
  let client: InstanceType<typeof Client>;
  const table = `tmp_txr_${Date.now() % 1_000_000}`;

  before(async () => {
    client = new Client(cfg());
    await client.connect();
    await client.query(`CREATE TABLE ${table} (id INT, val INT)`);
    await client.query(`INSERT INTO ${table} VALUES (1, 100)`);
  });
  after(async () => {
    await client.query(`DROP TABLE ${table}`).catch(() => {});
    await client.close();
  });

  it('rolled-back data is not visible', async () => {
    const tx = await client.beginTransaction();
    await tx.query(`DELETE FROM ${table} WHERE id = 1`);
    await tx.rollback();

    const r = await client.query(`SELECT COUNT(*) AS CNT FROM ${table}`);
    assert.equal(Number(r.rows[0].CNT), 1, 'row should still exist after rollback');
  });
});

describe('Transaction: transfer atomicity', () => {
  let client: InstanceType<typeof Client>;

  before(async () => {
    client = new Client(cfg());
    await client.connect();
  });
  after(async () => { await client.close(); });

  it('partial update is rolled back', async () => {
    const r1 = await client.query(
      'SELECT balance FROM accounts WHERE id = 1',
    );
    const startBalance = Number(r1.rows[0].BALANCE);

    const tx = await client.beginTransaction();
    await tx.query('UPDATE accounts SET balance = balance - 500 WHERE id = 1');
    await tx.rollback();

    const r2 = await client.query(
      'SELECT balance FROM accounts WHERE id = 1',
    );
    const endBalance = Number(r2.rows[0].BALANCE);
    assert.ok(
      Math.abs(startBalance - endBalance) < 0.01,
      'balance should be unchanged after rollback',
    );
  });
});

describe('Transaction: prepared statements', () => {
  let client: InstanceType<typeof Client>;

  before(async () => {
    client = new Client(cfg());
    await client.connect();
  });
  after(async () => { await client.close(); });

  it('keeps multiple prepared statements isolated inside one transaction', async () => {
    const tx = await client.beginTransaction();
    const stmt1 = await tx.prepare('VALUES CAST(? AS INTEGER)');
    const stmt2 = await tx.prepare('VALUES CAST(? AS INTEGER) + 100');

    try {
      const result1 = await stmt1.execute([1]);
      const result2 = await stmt2.execute([2]);

      assert.equal(Number(result1.rows[0].COL1 ?? result1.rows[0]['1']), 1);
      assert.equal(Number(result2.rows[0].COL1 ?? result2.rows[0]['1']), 102);
    } finally {
      await stmt1.close();
      await stmt2.close();
      await tx.rollback();
    }
  });
});
