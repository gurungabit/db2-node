/**
 * Query execution tests.
 * Ref: ibm_db test-query-select.js, test-querySync-select.js
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

describe('Query: SELECT', () => {
  let client: InstanceType<typeof Client>;

  before(async () => {
    client = new Client(cfg());
    await client.connect();
  });
  after(async () => { await client.close(); });

  it('SELECT 1 returns one row', async () => {
    const r = await client.query('SELECT 1 AS VAL FROM SYSIBM.SYSDUMMY1');
    assert.equal(r.rows.length, 1);
    assert.ok(r.columns.length >= 1);
  });

  it('SELECT multiple columns from employees', async () => {
    const r = await client.query(
      'SELECT id, name, salary FROM employees ORDER BY id',
    );
    assert.equal(r.rows.length, 5);
    assert.ok(r.rows[0].NAME !== undefined, 'NAME column should exist');
    assert.ok(r.rows[0].ID !== undefined, 'ID column should exist');
  });

  it('SELECT with WHERE returns filtered rows', async () => {
    const r = await client.query(
      "SELECT name FROM employees WHERE name = 'Alice'",
    );
    assert.equal(r.rows.length, 1);
  });

  it('SELECT with empty result returns 0 rows', async () => {
    const r = await client.query(
      'SELECT * FROM employees WHERE id = -999',
    );
    assert.equal(r.rows.length, 0);
  });

  it('SELECT with aggregate functions', async () => {
    const r = await client.query(
      'SELECT COUNT(*) AS CNT, AVG(salary) AS AVG_SAL FROM employees',
    );
    assert.equal(r.rows.length, 1);
    assert.ok(Number(r.rows[0].CNT) > 0);
  });

  it('SELECT with column aliases', async () => {
    const r = await client.query(
      'SELECT name AS emp_name, salary AS emp_salary FROM employees WHERE id = 1',
    );
    assert.equal(r.rows.length, 1);
    assert.ok(r.rows[0].EMP_NAME !== undefined || r.rows[0].emp_name !== undefined);
  });

  it('SELECT with ORDER BY and FETCH FIRST', async () => {
    const r = await client.query(
      'SELECT name FROM employees ORDER BY salary DESC FETCH FIRST 3 ROWS ONLY',
    );
    assert.equal(r.rows.length, 3);
  });
});

describe('Query: column metadata', () => {
  let client: InstanceType<typeof Client>;

  before(async () => {
    client = new Client(cfg());
    await client.connect();
  });
  after(async () => { await client.close(); });

  it('returns column names and types', async () => {
    const r = await client.query(
      'SELECT id, name, salary, hired FROM employees WHERE id = 1',
    );
    assert.ok(r.columns.length >= 4);
    const names = r.columns.map(c => c.name);
    assert.ok(names.includes('ID'));
    assert.ok(names.includes('NAME'));
    assert.ok(names.includes('SALARY'));
    assert.ok(names.includes('HIRED'));
  });

  it('reports nullable columns', async () => {
    const r = await client.query(
      'SELECT id, salary FROM employees WHERE id = 1',
    );
    const idCol = r.columns.find(c => c.name === 'ID');
    const salCol = r.columns.find(c => c.name === 'SALARY');
    assert.ok(idCol);
    assert.ok(salCol);
    assert.equal(idCol.nullable, false, 'ID is NOT NULL');
    assert.equal(salCol.nullable, true, 'SALARY is nullable');
  });
});

describe('Prepared statements', () => {
  let client: InstanceType<typeof Client>;

  before(async () => {
    client = new Client(cfg());
    await client.connect();
  });
  after(async () => { await client.close(); });

  it('keeps multiple prepared statements isolated on one connection', async () => {
    const stmt1 = await client.prepare('VALUES CAST(? AS INTEGER)');
    const stmt2 = await client.prepare('VALUES CAST(? AS INTEGER) + 100');

    try {
      const result1 = await stmt1.execute([1]);
      const result2 = await stmt2.execute([2]);

      assert.equal(Number(result1.rows[0].COL1 ?? result1.rows[0]['1']), 1);
      assert.equal(Number(result2.rows[0].COL1 ?? result2.rows[0]['1']), 102);
    } finally {
      await stmt1.close();
      await stmt2.close();
    }
  });
});
