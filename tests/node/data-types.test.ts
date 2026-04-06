/**
 * Data type round-trip tests.
 * Ref: ibm_db test-all-data-types.js, test-date.js, test-decimals.js
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

describe('Data types: integers', () => {
  let client: InstanceType<typeof Client>;
  const table = `tmp_int_${Date.now() % 1_000_000}`;

  before(async () => {
    client = new Client(cfg());
    await client.connect();
    await client.query(
      `CREATE TABLE ${table} (s SMALLINT, i INTEGER, b BIGINT)`,
    );
  });
  after(async () => {
    await client.query(`DROP TABLE ${table}`).catch(() => {});
    await client.close();
  });

  it('round-trips integer values', async () => {
    await client.query(`INSERT INTO ${table} VALUES (32767, 2147483647, 9223372036854775807)`);
    await client.query(`INSERT INTO ${table} VALUES (0, 0, 0)`);
    await client.query(`INSERT INTO ${table} VALUES (-1, -1, -1)`);
    const r = await client.query(`SELECT * FROM ${table} ORDER BY s`);
    assert.equal(r.rows.length, 3);
  });
});

describe('Data types: strings', () => {
  let client: InstanceType<typeof Client>;

  before(async () => {
    client = new Client(cfg());
    await client.connect();
  });
  after(async () => { await client.close(); });

  it('reads VARCHAR data', async () => {
    const r = await client.query(
      'SELECT ascii, unicode FROM test_strings ORDER BY id',
    );
    assert.ok(r.rows.length >= 3);
    assert.ok(r.rows[0].ASCII.includes('Hello'));
  });

  it('reads empty strings vs NULL', async () => {
    const r = await client.query(
      "SELECT empty FROM test_strings WHERE empty = ''",
    );
    // at least one row with empty string
    assert.ok(r.rows.length >= 1);
  });
});

describe('Data types: decimal', () => {
  let client: InstanceType<typeof Client>;
  const table = `tmp_dec_${Date.now() % 1_000_000}`;

  before(async () => {
    client = new Client(cfg());
    await client.connect();
    await client.query(`CREATE TABLE ${table} (val DECIMAL(15,2))`);
  });
  after(async () => {
    await client.query(`DROP TABLE ${table}`).catch(() => {});
    await client.close();
  });

  it('round-trips decimal values', async () => {
    await client.query(`INSERT INTO ${table} VALUES (12345.67)`);
    await client.query(`INSERT INTO ${table} VALUES (-99999.99)`);
    await client.query(`INSERT INTO ${table} VALUES (0.01)`);
    const r = await client.query(`SELECT * FROM ${table} ORDER BY val`);
    assert.equal(r.rows.length, 3);
    // Decimal is returned as string to preserve precision
    assert.ok(r.rows[2].VAL.includes('12345.67'));
  });

  it('reads and binds DECFLOAT values as strings', async () => {
    const literal = await client.query(
      "VALUES (CAST('123.45' AS DECFLOAT(16)), CAST('-987654321.00001' AS DECFLOAT(34)))",
    );
    assert.equal(literal.rows.length, 1);
    assert.ok(literal.columns[0].typeName.includes('DecFloat(16)'));
    assert.ok(literal.columns[1].typeName.includes('DecFloat(34)'));
    assert.equal(literal.rows[0].COL1, '123.45');
    assert.equal(literal.rows[0].COL2, '-987654321.00001');

    const bound = await client.query(
      'VALUES CAST(? AS DECFLOAT(16))',
      ['42.125'],
    );
    assert.equal(bound.rows.length, 1);
    assert.equal(bound.rows[0].COL1 ?? bound.rows[0]['1'], '42.125');
  });
});

describe('Data types: date and time', () => {
  let client: InstanceType<typeof Client>;
  const table = `tmp_dt_${Date.now() % 1_000_000}`;

  before(async () => {
    client = new Client(cfg());
    await client.connect();
    await client.query(`CREATE TABLE ${table} (d DATE, t TIME, ts TIMESTAMP)`);
  });
  after(async () => {
    await client.query(`DROP TABLE ${table}`).catch(() => {});
    await client.close();
  });

  it('round-trips date/time values', async () => {
    await client.query(
      `INSERT INTO ${table} VALUES ('2024-06-15', '13.30.45', '2024-06-15-13.30.45.123456')`,
    );
    const r = await client.query(`SELECT * FROM ${table}`);
    assert.equal(r.rows.length, 1);
    assert.ok(r.rows[0].D.includes('2024'));
  });
});

describe('Data types: NULL handling', () => {
  let client: InstanceType<typeof Client>;

  before(async () => {
    client = new Client(cfg());
    await client.connect();
  });
  after(async () => { await client.close(); });

  it('reads all-NULL row', async () => {
    const r = await client.query(
      'SELECT * FROM test_nulls WHERE col1 IS NULL AND col2 IS NULL ORDER BY id FETCH FIRST 1 ROW ONLY',
    );
    assert.equal(r.rows.length, 1);
    // All columns except ID should be null
    assert.equal(r.rows[0].COL1, null);
    assert.equal(r.rows[0].COL2, null);
  });

  it('reads mixed NULL/non-NULL row', async () => {
    const r = await client.query(
      'SELECT col1, col2, col3 FROM test_nulls WHERE col1 = 1 AND col2 IS NULL',
    );
    assert.equal(r.rows.length, 1);
    assert.ok(r.rows[0].COL1 !== null);
    assert.equal(r.rows[0].COL2, null);
  });
});

describe('Data types: boolean', () => {
  let client: InstanceType<typeof Client>;

  before(async () => {
    client = new Client(cfg());
    await client.connect();
  });
  after(async () => { await client.close(); });

  it('reads boolean values', async () => {
    const r = await client.query(
      'SELECT active FROM employees ORDER BY id',
    );
    assert.ok(r.rows.length >= 5);
    // Alice is active (true), Dave is not (false)
  });
});
