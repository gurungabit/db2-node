/**
 * DDL and DML tests.
 * Ref: ibm_db test-query-insert.js, test-query-create.js
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

describe('DDL: CREATE / DROP TABLE', () => {
  let client: InstanceType<typeof Client>;
  const table = `tmp_ddl_${Date.now() % 1_000_000}`;

  before(async () => {
    client = new Client(cfg());
    await client.connect();
  });
  after(async () => {
    await client.query(`DROP TABLE IF EXISTS ${table}`).catch(() => {});
    await client.close();
  });

  it('CREATE TABLE succeeds', async () => {
    await client.query(
      `CREATE TABLE ${table} (id INTEGER NOT NULL, name VARCHAR(50), score DECIMAL(5,2))`,
    );
    // verify by querying it
    const r = await client.query(`SELECT * FROM ${table}`);
    assert.equal(r.rows.length, 0);
    assert.ok(r.columns.length >= 3);
  });

  it('DROP TABLE succeeds', async () => {
    await client.query(`DROP TABLE ${table}`);
    // re-create for cleanup handler
    await assert.rejects(
      () => client.query(`SELECT * FROM ${table}`),
      'querying dropped table should fail',
    );
  });
});

describe('DML: INSERT / UPDATE / DELETE', () => {
  let client: InstanceType<typeof Client>;
  const table = `tmp_dml_${Date.now() % 1_000_000}`;

  before(async () => {
    client = new Client(cfg());
    await client.connect();
    await client.query(
      `CREATE TABLE ${table} (id INTEGER, name VARCHAR(50), val INTEGER)`,
    );
  });
  after(async () => {
    await client.query(`DROP TABLE ${table}`).catch(() => {});
    await client.close();
  });

  it('INSERT rows and verify with SELECT', async () => {
    await client.query(`INSERT INTO ${table} VALUES (1, 'Alice', 100)`);
    await client.query(`INSERT INTO ${table} VALUES (2, 'Bob', 200)`);
    await client.query(`INSERT INTO ${table} VALUES (3, 'Carol', 300)`);

    const r = await client.query(`SELECT * FROM ${table} ORDER BY id`);
    assert.equal(r.rows.length, 3);
  });

  it('UPDATE modifies rows', async () => {
    await client.query(`UPDATE ${table} SET val = val + 10 WHERE id <= 2`);
    const r = await client.query(
      `SELECT val FROM ${table} WHERE id = 1`,
    );
    assert.equal(r.rows.length, 1);
    assert.equal(Number(r.rows[0].VAL), 110);
  });

  it('DELETE removes rows', async () => {
    await client.query(`DELETE FROM ${table} WHERE id = 3`);
    const r = await client.query(`SELECT * FROM ${table}`);
    assert.equal(r.rows.length, 2);
  });

  it('INSERT with multi-row VALUES', async () => {
    await client.query(
      `INSERT INTO ${table} VALUES (10, 'X', 1), (11, 'Y', 2), (12, 'Z', 3)`,
    );
    const r = await client.query(`SELECT * FROM ${table} WHERE id >= 10`);
    assert.equal(r.rows.length, 3);
  });
});
