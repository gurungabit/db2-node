import test from 'node:test';
import assert from 'node:assert/strict';
import net from 'node:net';
import { randomBytes } from 'node:crypto';
import db2 from '../../crates/db2-napi/index.cjs';
import { getConfig } from './helpers';

const config = { ...getConfig(), host: process.env.DB2_TEST_HOST || '127.0.0.1' };
const arrayMode = { rowMode: 'array' } as const;
const uniqueName = () => `ISSUE2123_${process.pid}_${randomBytes(6).toString('hex').toUpperCase()}`;
const duplicateSql = 'SELECT 11 AS A, 22 AS A, CAST(NULL AS INTEGER) AS N FROM SYSIBM.SYSDUMMY1';

function checkArray(result: any) {
  assert.deepEqual(result.rows, [[11, 22, null]]);
  assert.deepEqual(result.columns.map((column: any) => column.name), ['A', 'A', 'N']);
  assert.equal(result.rowCount, 1);
}

function driverError(code: string) {
  return (error: any) => {
    assert.ok(error instanceof Error);
    assert.equal(error.driverCode, code);
    assert.equal(error.code, 'GenericFailure');
    assert.equal(error.sqlstate, undefined);
    assert.equal(error.sqlcode, undefined);
    assert.equal(error.retryable ?? false, false);
    return true;
  };
}

test('issues #21-23: query API regressions', { timeout: 180_000 }, async (t) => {
  for (const Client of [db2.Client, db2.NativeClient]) {
    await t.test(`${Client.name}: ordered arrays, default objects, statements and transactions`, async () => {
      const client = new Client(config);
      await client.connect();
      try {
        assert.deepEqual((await client.query(duplicateSql)).rows, [{ A: 22, N: null }]);
        assert.deepEqual((await client.query(duplicateSql, [], { rowMode: 'object' })).rows, [{ A: 22, N: null }]);
        checkArray(await client.query(duplicateSql, undefined, arrayMode));
        assert.deepEqual((await client.query(`${duplicateSql} WHERE 1=0`, null, arrayMode)).rows, []);
        // Numeric column names cannot be recovered in SELECT order via Object.values().
        const mixed = await client.query(`SELECT 10 AS "10", 2 AS "2", FALSE AS B,
          CAST('é' AS VARCHAR(10)) AS S, CAST(1.2 AS DECIMAL(5,2)) AS D,
          CAST(9223372036854775807 AS BIGINT) AS I, CAST(X'00FF' AS VARBINARY(2)) AS X
          FROM SYSIBM.SYSDUMMY1`, [], arrayMode);
        assert.deepEqual(mixed.rows, [[10, 2, false, 'é', '1.20', '9223372036854775807', Buffer.from([0, 255])]]);
        const statement = await client.prepare(duplicateSql);
        try {
          checkArray(await statement.execute(null, arrayMode));
          assert.deepEqual((await statement.execute()).rows, [{ A: 22, N: null }]);
        } finally { await statement.close(); }
        const tx = await client.beginTransaction();
        try {
          checkArray(await tx.query(duplicateSql, [], arrayMode));
          const prepared = await tx.prepare(duplicateSql);
          try { checkArray(await prepared.execute(undefined, arrayMode)); }
          finally { await prepared.close(); }
        } finally { await tx.rollback(); }
        await assert.rejects(() => client.query(duplicateSql, [], { rowMode: 'invalid' }), driverError('DB2_INVALID_OPTION'));
      } finally { await client.close(); }
    });

    await t.test(`${Client.name}: Db2 BOOLEAN aliases and classified parameter failures`, async () => {
      const client = new Client(config);
      const table = uniqueName();
      await client.connect();
      await client.query(`CREATE TABLE ${table}(ID INTEGER, B BOOLEAN)`);
      try {
        await client.query(`INSERT INTO ${table} VALUES(1,TRUE)`);
        const cast = await client.prepare('VALUES CAST(? AS BOOLEAN)');
        const update = await client.prepare(`UPDATE ${table} SET B=? WHERE ID=1`);
        try {
          for (const [expected, words] of [[true, ['t', 'true', 'y', 'yes', 'on', '1']], [false, ['f', 'false', 'n', 'no', 'off', '0']]] as const) {
            for (const word of words) {
              const value = ` ${word.toUpperCase()} `;
              assert.equal((await client.query(`VALUES CAST('${value}' AS BOOLEAN)`)).rows[0]['1'], expected);
              assert.equal((await client.query('VALUES CAST(? AS BOOLEAN)', [value])).rows[0]['1'], expected);
              assert.equal((await cast.execute([value])).rows[0]['1'], expected);
              await update.execute([value]);
              assert.equal((await client.query(`SELECT B FROM ${table}`)).rows[0].B, expected);
            }
          }
          for (const [value, expected] of [[true, true], [false, false], [0, false], [1, true], [-2, true], [null, null]] as const) {
            assert.equal((await cast.execute([value])).rows[0]['1'], expected);
          }
          assert.equal((await client.query('VALUES CAST(? AS VARCHAR(20))', ['false'])).rows[0]['1'], 'false');
          assert.equal((await client.query('VALUES CAST(CAST(? AS VARCHAR(128)) AS BOOLEAN)', ['2'])).rows[0]['1'], true);
          for (const value of ['truth', '', 'true false', '\ttrue\t', '\nfalse\n', 'SQLSTATE=26501, SQLCODE=-514 QRYNOPRM closed by server']) {
            await assert.rejects(() => client.query('VALUES CAST(? AS BOOLEAN)', [value]), driverError('DB2_PARAMETER_TYPE'));
            await assert.rejects(() => update.execute([value]), driverError('DB2_PARAMETER_TYPE'));
            assert.equal((await client.query(`SELECT B FROM ${table}`)).rows[0].B, false);
          }
          for (const params of [[], [1, 2]]) {
            await assert.rejects(() => cast.execute(params), driverError('DB2_PARAMETER_COUNT'));
          }
          await assert.rejects(() => client.query('VALUES CAST(? AS INTEGER)', [1, 2]), driverError('DB2_PARAMETER_COUNT'));
          await assert.rejects(() => client.query('VALUES CAST(? AS INTEGER)', ['abc']), driverError('DB2_PARAMETER_TYPE'));
          await assert.rejects(() => client.query('VALUES CAST(? AS DECIMAL(5,2))', ['abc']), driverError('DB2_PARAMETER_TYPE'));
          for (const value of [{}, [256], [[1]], Symbol('unsupported'), () => 1, new Date(NaN)]) {
            await assert.rejects(async () => client.query('VALUES CAST(? AS BOOLEAN)', [value]), (error: any) => {
              assert.equal(error.driverCode, 'DB2_PARAMETER_TYPE');
              return true;
            });
          }
          await assert.rejects(() => update.executeBatch([['invalid']], arrayMode), driverError('DB2_PARAMETER_TYPE'));
          await assert.rejects(() => update.executeBatch([[true, 2]]), driverError('DB2_PARAMETER_COUNT'));
          const batch = await update.executeBatch([['true'], ['false']], arrayMode);
          assert.deepEqual(batch.rows, []);
          assert.equal(batch.rowCount, 2);
        } finally { await cast.close(); await update.close(); }
        const tx = await client.beginTransaction();
        try {
          assert.equal((await tx.query('VALUES CAST(? AS BOOLEAN)', ['false'])).rows[0]['1'], false);
          await assert.rejects(() => tx.query('VALUES CAST(? AS BOOLEAN)', ['invalid']), driverError('DB2_PARAMETER_TYPE'));
          await assert.rejects(() => tx.query('VALUES CAST(? AS INTEGER)', [1, 2]), driverError('DB2_PARAMETER_COUNT'));
        } finally { await tx.rollback(); }
        await assert.rejects(() => client.query('SELECT * FROM DB2_NO_TABLE_2123'), (error: any) => {
          assert.equal(error.driverCode, undefined);
          assert.equal(error.code, 'GenericFailure');
          assert.match(error.message, /SQLSTATE=42704, SQLCODE=-204/);
          if (Client === db2.Client) {
            assert.equal(error.sqlstate, '42704');
            assert.equal(error.sqlcode, -204);
            assert.equal(error.retryable, false);
          }
          return true;
        });
        assert.deepEqual((await client.query('VALUES 1')).rows, [{ '1': 1 }]);
      } finally { await client.query(`DROP TABLE ${table}`); await client.close(); }
    });
  }

  for (const Pool of [db2.Pool, db2.Db2Pool, db2.NativePool]) {
    await t.test(`${Pool.name}: pooled arrays and errors`, async () => {
      const pool = new Pool({ ...config, minConnections: 1, maxConnections: 1 });
      try {
        await pool.connect();
        checkArray(await pool.query(duplicateSql, [], arrayMode));
        assert.deepEqual((await pool.query('VALUES CAST(? AS BOOLEAN)', ['false'], arrayMode)).rows, [[false]]);
        assert.deepEqual((await pool.query(duplicateSql)).rows, [{ A: 22, N: null }]);
        await assert.rejects(() => pool.query('VALUES CAST(? AS BOOLEAN)', ['invalid']), driverError('DB2_PARAMETER_TYPE'));
        await assert.rejects(() => pool.query('VALUES CAST(? AS INTEGER)', [1, 2]), driverError('DB2_PARAMETER_COUNT'));
        const acquired = await pool.acquire();
        try { checkArray(await acquired.query(duplicateSql, null, arrayMode)); }
        finally { await pool.release(acquired); }
        if (Pool === db2.Pool) {
          await new Promise<void>((resolve, reject) => pool.query(duplicateSql, [], arrayMode, (err: any, result: any) => {
            if (err) { reject(err); return; }
            try { checkArray(result); resolve(); } catch (err) { reject(err); }
          }));
          await new Promise<void>((resolve, reject) => pool.query(duplicateSql, [], (err: any, result: any) => {
            if (err) { reject(err); return; }
            try { assert.deepEqual(result.rows, [{ A: 22, N: null }]); resolve(); } catch (err) { reject(err); }
          }));
        }
      } finally { await pool.close(); }
    });
  }

  await t.test('CALL arrays apply to every result set and preserve output parameters', async () => {
    const client = new db2.Client(config);
    const procedure = uniqueName();
    await client.connect();
    await client.query(`CREATE PROCEDURE ${procedure}(OUT O INTEGER)
      LANGUAGE SQL DYNAMIC RESULT SETS 2 BEGIN
      DECLARE C1 CURSOR WITH RETURN TO CLIENT FOR ${duplicateSql};
      DECLARE C2 CURSOR WITH RETURN TO CLIENT FOR SELECT 33 AS Z, 44 AS Z FROM SYSIBM.SYSDUMMY1;
      SET O=7; OPEN C1; OPEN C2; END`);
    try {
      const verify = (result: any) => {
        checkArray(result);
        assert.equal(result.resultSets.length, 2);
        checkArray(result.resultSets[0]);
        assert.deepEqual(result.resultSets[1].rows, [[33, 44]]);
        assert.deepEqual(result.resultSets[1].columns.map((c: any) => c.name), ['Z', 'Z']);
        assert.deepEqual(result.outputParameters, [7]);
      };
      verify(await client.query(`CALL ${procedure}(?)`, [null], arrayMode));
      const statement = await client.prepare(`CALL ${procedure}(?)`);
      try { verify(await statement.execute([null], arrayMode)); } finally { await statement.close(); }
      const tx = await client.beginTransaction();
      try { verify(await tx.query(`CALL ${procedure}(?)`, [null], arrayMode)); } finally { await tx.rollback(); }
      const database = await db2.open(config);
      try {
        assert.deepEqual(await database.query({ sql: duplicateSql, rowMode: 'array' }), [[11, 22, null]]);
        verify(await database.queryResult({ sql: `CALL ${procedure}(?)`, params: [null], rowMode: 'array' }));
        const rows = [];
        for await (const row of database.queryStream({ sql: duplicateSql, rowMode: 'array' })) rows.push(row);
        assert.deepEqual(rows, [[11, 22, null]]);
      } finally { await database.close(); }
    } finally { await client.query(`DROP PROCEDURE ${procedure}`); await client.close(); }
  });
});

test('issue #23: malformed peer replies carry DB2_PROTOCOL', { timeout: 15_000 }, async () => {
  const sockets = new Set<net.Socket>();
  const server = net.createServer((socket) => {
    sockets.add(socket);
    socket.once('data', () => socket.end(Buffer.from([0, 6, 0, 2, 0, 1])));
    socket.on('error', () => {});
    socket.on('close', () => sockets.delete(socket));
  });
  await new Promise<void>((resolve) => server.listen(0, '127.0.0.1', resolve));
  try {
    for (const Client of [db2.Client, db2.NativeClient]) {
      const client = new Client({ ...config, port: (server.address() as net.AddressInfo).port, connectTimeout: 2000 });
      try { await assert.rejects(() => client.connect(), driverError('DB2_PROTOCOL')); }
      finally { await client.close(); }
    }
  } finally {
    for (const socket of sockets) socket.destroy();
    await new Promise<void>((resolve) => server.close(() => resolve()));
  }
});
