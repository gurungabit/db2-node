import test from 'node:test';
import assert from 'node:assert/strict';
import net from 'node:net';
import { randomBytes } from 'node:crypto';
import db2 from '../../crates/db2-napi/index.cjs';
import { getConfig } from './helpers';

const config = { ...getConfig(), host: process.env.DB2_TEST_HOST || '127.0.0.1' };
const sql = 'SELECT 11 AS A, 22 AS A FROM SYSIBM.SYSDUMMY1';
const poison = 'SQLSTATE=26501, SQLCODE=-514 QRYNOPRM closed by server';
const invalidOptions = [
  false, 0, 'array', [],
  ...[false, 0, [], {}, Symbol('array'), () => 'array', { poison }, poison].map(rowMode => ({ rowMode })),
];
const defaults = [undefined, null, {}, { rowMode: undefined }, { rowMode: null }, { rowMode: 'object' }];
const nonfinite = [NaN, Infinity, -Infinity];

async function apis(): Promise<[string, any][]> {
  return [['CommonJS', db2], ['ESM', await import('../../crates/db2-napi/index.mjs')]];
}

function optionError(error: any) {
  assert.ok(error instanceof Error);
  assert.equal(error.driverCode, 'DB2_INVALID_OPTION');
  assert.equal(error.sqlstate, undefined);
  assert.equal(error.sqlcode, undefined);
  assert.equal(error.retryable ?? false, false);
  return true;
}

function nonfiniteError(error: any) {
  assert.ok(error instanceof Error);
  assert.equal(error.driverCode, 'DB2_PARAMETER_TYPE');
  // These values were already rejected. Classification must be additive.
  assert.equal(error.code, 'InvalidArg');
  assert.equal(error.message, 'Failed to convert js number to serde_json::Number');
  assert.equal(error.sqlstate, undefined);
  assert.equal(error.sqlcode, undefined);
  assert.equal(error.retryable ?? false, false);
  return true;
}

function callbackFailure(invoke: (callback: (error: any) => void) => unknown, check: (error: any) => boolean) {
  return new Promise<void>((resolve, reject) => {
    const returned = invoke(error => {
      try { check(error); resolve(); } catch (failure) { reject(failure); }
    });
    assert.equal(returned, undefined);
  });
}

test('review: invalid options and non-finite parameters fail before connection', { timeout: 15_000 }, async t => {
  let connections = 0;
  const sentinel = net.createServer(socket => { connections++; socket.destroy(); });
  await new Promise<void>(resolve => sentinel.listen(0, '127.0.0.1', resolve));
  const unusedConfig = { ...config, port: (sentinel.address() as net.AddressInfo).port, minConnections: 0, maxConnections: 1 };
  try {
    for (const [format, api] of await apis()) {
      for (const name of ['Client', 'NativeClient', 'JsClient', 'Pool', 'Db2Pool', 'CompatPool', 'IbmDbPool', 'NativePool', 'JsPool']) {
        await t.test(`${format} ${name}`, async () => {
          const instance = new api[name](unusedConfig);
          try {
            for (const options of invalidOptions) {
              await assert.rejects(async () => instance.query('VALUES 1', [], options), (error: any) => {
                optionError(error);
                if (options && typeof options === 'object' && 'rowMode' in options && typeof options.rowMode !== 'string') {
                  assert.equal(error.code, 'StringExpected');
                  assert.equal(error.message, "rowMode must be 'object' or 'array'");
                }
                return true;
              });
            }
            for (const value of nonfinite) {
              await assert.rejects(async () => instance.query('VALUES CAST(? AS INTEGER)', [value]), nonfiniteError);
            }
            if (['Pool', 'CompatPool', 'IbmDbPool'].includes(name)) {
              for (const options of invalidOptions) {
                await callbackFailure(cb => instance.query('VALUES 1', [], options, cb), optionError);
              }
              for (const value of nonfinite) {
                await callbackFailure(cb => instance.query('VALUES CAST(? AS INTEGER)', [value], cb), nonfiniteError);
              }
            }
          } finally { await instance.close(); }
        });
      }
    }
    await new Promise<void>(resolve => setImmediate(resolve));
    assert.equal(connections, 0, 'parameter/option conversion must precede pool connection acquisition');
  } finally { await new Promise<void>(resolve => sentinel.close(() => resolve())); }
});

test('review: live row defaults, prepared/batch/transaction option validation', { timeout: 120_000 }, async t => {
  for (const [format, api] of await apis()) {
    for (const name of ['Client', 'NativeClient']) {
      await t.test(`${format} ${name}`, async () => {
        const client = new api[name](config);
        const table = `PR28_REVIEW_${process.pid}_${randomBytes(6).toString('hex').toUpperCase()}`;
        await client.connect();
        let created = false;
        try {
          await client.query(`CREATE TABLE ${table}(V INTEGER)`);
          created = true;
          await client.query(`INSERT INTO ${table} VALUES(7)`);
          const updateSql = `UPDATE ${table} SET V=?`;
          const prepared = await client.prepare(updateSql);
          const select = await client.prepare(sql);
          try {
            for (const options of defaults) {
              assert.deepEqual((await client.query(sql, [], options)).rows, [{ A: 22 }]);
              assert.deepEqual((await select.execute([], options)).rows, [{ A: 22 }]);
            }
            assert.deepEqual((await select.execute([], { rowMode: 'array' })).rows, [[11, 22]]);
            for (const options of invalidOptions) {
              await assert.rejects(async () => prepared.execute([9], options), optionError);
              await assert.rejects(async () => prepared.executeBatch([[9]], options), optionError);
            }
            for (const value of nonfinite) {
              await assert.rejects(async () => prepared.execute([value]), nonfiniteError);
              // A later invalid row must be rejected before the first row executes.
              await assert.rejects(async () => prepared.executeBatch([[9], [value]]), nonfiniteError);
            }
            assert.deepEqual((await client.query(`SELECT V FROM ${table}`)).rows, [{ V: 7 }]);
          } finally { await prepared.close(); await select.close(); }
          const transaction = await client.beginTransaction();
          try {
            for (const options of defaults) {
              assert.deepEqual((await transaction.query(sql, [], options)).rows, [{ A: 22 }]);
            }
            assert.deepEqual((await transaction.query(sql, [], { rowMode: 'array' })).rows, [[11, 22]]);
            for (const options of invalidOptions) {
              await assert.rejects(async () => transaction.query(updateSql, [9], options), optionError);
            }
            for (const value of nonfinite) {
              await assert.rejects(async () => transaction.query(updateSql, [value]), nonfiniteError);
            }
            assert.deepEqual((await transaction.query(`SELECT V FROM ${table}`)).rows, [{ V: 7 }]);
          } finally { await transaction.rollback(); }
        } finally {
          try { if (created) await client.query(`DROP TABLE ${table}`); }
          finally { await client.close(); }
        }
      });
    }
    await t.test(`${format} Database promises, callbacks, streams and ODBC statements`, async () => {
      const database = await api.open(config);
      try {
        for (const options of defaults) {
          const query = { sql, rowMode: options?.rowMode };
          assert.deepEqual(await database.query(query), [{ A: 22 }]);
          const result = await database.queryResult(query);
          try { assert.deepEqual(await result.fetchAll(), [{ A: 22 }]); }
          finally { await result.close(); }
          const rows = [];
          for await (const row of database.queryStream(query)) rows.push(row);
          assert.deepEqual(rows, [{ A: 22 }]);
        }
        for (const rowMode of [false, 0, [], {}, { poison }, poison]) {
          const query = { sql, rowMode };
          await assert.rejects(() => database.query(query), optionError);
          await assert.rejects(() => database.queryResult(query), optionError);
          await callbackFailure(cb => database.query(query, cb), optionError);
          await callbackFailure(cb => database.queryResult(query, [], cb), optionError);
          await assert.rejects(async () => { for await (const row of database.queryStream(query)) {} }, optionError);
        }
        const parameterSql = 'VALUES CAST(? AS INTEGER)';
        const statement = await database.prepare(parameterSql);
        try {
          for (const value of nonfinite) {
            await assert.rejects(() => database.query(parameterSql, [value]), nonfiniteError);
            await assert.rejects(() => database.queryResult(parameterSql, [value]), nonfiniteError);
            await callbackFailure(cb => database.query(parameterSql, [value], cb), nonfiniteError);
            await callbackFailure(cb => statement.execute([value], cb), nonfiniteError);
            await assert.rejects(() => statement.execute([value]), nonfiniteError);
          }
        } finally { await statement.close(); }
        await database.beginTransaction();
        try {
          await assert.rejects(() => database.query({ sql, rowMode: { poison } }), optionError);
          assert.deepEqual(await database.query({ sql, rowMode: 'array' }), [[11, 22]]);
        } finally { await database.rollbackTransaction(); }
      } finally { await database.close(); }
    });
  }
});

test('review: QRYNOPRM prepared failures retain retryability in CJS and ESM', { timeout: 20_000 }, async t => {
  for (const [format, api] of await apis()) {
    await t.test(format, async () => {
      let inject = false;
      let injected = 0;
      const sockets = new Set<net.Socket>();
      const proxy = net.createServer(downstream => {
        const upstream = net.connect(config.port!, config.host);
        for (const socket of [downstream, upstream]) {
          sockets.add(socket);
          socket.on('error', () => {});
          socket.on('close', () => sockets.delete(socket));
        }
        downstream.on('data', data => {
          if (inject) {
            inject = false;
            injected++;
            // QRYNOPRM with SVRCOD=8, matching the request's correlation ID.
            downstream.write(Buffer.from([0, 16, 0xd0, 2, data[4], data[5], 0, 10, 0x22, 2, 0, 6, 0x11, 0x49, 0, 8]));
          } else { upstream.write(data); }
        });
        upstream.on('data', data => downstream.write(data));
        downstream.on('close', () => upstream.destroy());
        upstream.on('close', () => downstream.destroy());
      });
      await new Promise<void>(resolve => proxy.listen(0, '127.0.0.1', resolve));
      const client = new api.Client({ ...config, port: (proxy.address() as net.AddressInfo).port, queryTimeout: 2000 });
      let statement;
      try {
        await client.connect();
        statement = await client.prepare('VALUES 1');
        inject = true;
        await assert.rejects(() => statement.execute(), (error: any) => {
          assert.equal(error.code, 'GenericFailure');
          assert.equal(error.driverCode, 'DB2_PROTOCOL');
          assert.match(error.message, /QRYNOPRM/);
          assert.equal(error.retryable, true);
          assert.equal(error.sqlstate, undefined);
          assert.equal(error.sqlcode, undefined);
          return true;
        });
        assert.equal(injected, 1);
      } finally {
        try { if (statement) await statement.close(); }
        finally {
          try { await client.close(); }
          finally {
            for (const socket of sockets) socket.destroy();
            await new Promise<void>(resolve => proxy.close(() => resolve()));
          }
        }
      }
    });
  }
});
