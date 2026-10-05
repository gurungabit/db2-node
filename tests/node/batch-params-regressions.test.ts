import test from 'node:test';
import assert from 'node:assert/strict';
import { Client, getConfig } from './helpers';

const uniqueTable = (name: string) => `BPR_${name}_${process.pid}_${process.hrtime.bigint()}`;

test('safe integer numbers bind to BIGINT, DOUBLE and DECIMAL', async () => {
  const client = new Client(getConfig());
  await client.connect();
  const table = uniqueTable('BIGINT');
  try {
    await client.query(`CREATE TABLE ${table} (ID INTEGER NOT NULL, B BIGINT, D DOUBLE, P DECIMAL(16,0))`);
    try {
      // napi kept only u32-sized integers integral; larger ones arrived as DOUBLE.
      const values = [606_227_179_000, Number.MAX_SAFE_INTEGER, -Number.MAX_SAFE_INTEGER, 2 ** 32, -(2 ** 31) - 1, 0];
      for (const [index, value] of values.entries()) {
        assert.equal((await client.query(`INSERT INTO ${table} VALUES (?, ?, ?, ?)`, [index, value, value, value])).rowCount, 1);
      }
      const insert = await client.prepare(`INSERT INTO ${table} VALUES (?, ?, ?, ?)`);
      try {
        assert.equal((await insert.executeBatch(values.map((value, index) => [index + 100, value, value, value]))).rowCount, values.length);
      } finally { await insert.close(); }
      const rows = (await client.query(`SELECT B, D, P FROM ${table} ORDER BY ID`, [], { rowMode: 'array' })).rows;
      const expected = values.map(value => [value, value, String(value)]);
      assert.deepEqual(rows, [...expected, ...expected]);

      await assert.rejects(async () => client.query(`INSERT INTO ${table} (ID, B) VALUES (?, ?)`, [99, 1.5]), /integer-compatible/);
      assert.deepEqual((await client.query('VALUES CAST(? AS DOUBLE)', [1.5])).rows, [{ '1': 1.5 }]);
    } finally { await client.query(`DROP TABLE ${table}`); }
  } finally { await client.close(); }
});

test('executeBatch commits under autocommit and rolls back a failed batch', async () => {
  const client = new Client(getConfig());
  const observer = new Client(getConfig());
  await client.connect();
  await observer.connect();
  const table = uniqueTable('BATCH');
  const count = async (c: InstanceType<typeof Client>) => (await c.query(`SELECT COUNT(*) AS N FROM ${table}`)).rows[0].N;
  try {
    await client.query(`CREATE TABLE ${table} (ID INTEGER NOT NULL PRIMARY KEY)`);
    try {
      const insert = await client.prepare(`INSERT INTO ${table} VALUES (?)`);
      try {
        assert.equal((await insert.executeBatch([[1], [2], [3]])).rowCount, 3);
        assert.equal(await count(observer), 3);

        // Row 3 duplicates a key: no row of the failed batch is kept.
        await assert.rejects(insert.executeBatch([[4], [5], [1], [6]]), { sqlstate: '23505' });
        assert.equal(await count(observer), 3);
        assert.equal(await count(client), 3);
      } finally { await insert.close(); }

      const tx = await client.beginTransaction();
      const txInsert = await tx.prepare(`INSERT INTO ${table} VALUES (?)`);
      assert.equal((await txInsert.executeBatch([[10], [11]])).rowCount, 2);
      await txInsert.close();
      assert.equal(await count(observer), 3);
      await tx.commit();
      assert.equal(await count(observer), 5);
    } finally { await client.query(`DROP TABLE ${table}`); }
  } finally {
    await observer.close();
    await client.close();
  }
});

test('statements accept more than 84 parameters', { timeout: 60_000 }, async () => {
  const client = new Client(getConfig());
  await client.connect();
  const table = uniqueTable('WIDE');
  try {
    for (const n of [85, 169, 500]) {
      const ids = Array.from({ length: n }, (_, index) => index + 1);
      const sum = (await client.query(`VALUES ${ids.map(() => 'CAST(? AS INTEGER)').join(' + ')}`, ids)).rows[0]['1'];
      assert.equal(sum, (n * (n + 1)) / 2, `${n}`);
    }

    // 100 columns, with an externalized CLOB after the first 84 descriptors.
    const columns = Array.from({ length: 100 }, (_, index) => index === 89 ? `C${index + 1} CLOB(1M)` : `C${index + 1} INTEGER`);
    await client.query(`CREATE TABLE ${table} (${columns.join(', ')})`);
    try {
      const clob = 'w'.repeat(40_000);
      const row = (id: number) => Array.from({ length: 100 }, (_, index) => index === 89 ? clob : id * 1000 + index);
      const insert = await client.prepare(`INSERT INTO ${table} VALUES (${Array(100).fill('?').join(', ')})`);
      try {
        assert.equal((await insert.executeBatch([row(1), row(2)])).rowCount, 2);
      } finally { await insert.close(); }
      assert.deepEqual((await client.query(`SELECT C1, C89, LENGTH(C90) AS L90, C100 FROM ${table} ORDER BY C1`, [], { rowMode: 'array' })).rows, [
        [1000, 1088, 40_000, 1099],
        [2000, 2088, 40_000, 2099],
      ]);
      const ids = Array.from({ length: 300 }, (_, index) => index * 1000);
      assert.deepEqual((await client.query(`SELECT C1 FROM ${table} WHERE C1 IN (${ids.map(() => '?').join(', ')}) ORDER BY C1`, ids)).rows, [{ C1: 1000 }, { C1: 2000 }]);
    } finally { await client.query(`DROP TABLE ${table}`); }
  } finally { await client.close(); }
});
