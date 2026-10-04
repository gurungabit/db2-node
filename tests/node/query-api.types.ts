// Compile with tsc --strict --noEmit; these calls are never executed.
import {
  Client, Pool, Db2Pool, CompatPool, NativeClient, NativePool,
  JsClient, JsPool, PreparedStatement, NativePreparedStatement,
  Transaction, NativeTransaction,
} from '../../crates/db2-napi';
import type { QueryOptions, QueryResult, Db2Error, RowMode } from '../../crates/db2-napi';

async function types(
  clients: [Client, Pool, Db2Pool, CompatPool, NativeClient, NativePool, JsClient, JsPool, Transaction, NativeTransaction],
  statements: [PreparedStatement, NativePreparedStatement],
  options: QueryOptions,
) {
  for (const client of clients) {
    const objects: QueryResult = await client.query('VALUES 1');
    const arrays: QueryResult<'array'> = await client.query('VALUES 1', null, { rowMode: 'array' });
    const array: any[] = arrays.rows[0];
    const nested: any[] = arrays.resultSets[0].rows[0];
    const dynamic: QueryResult<RowMode> = await client.query('VALUES 1', [], options);
    // @ts-expect-error an array result cannot be treated as object rows
    const wrong: QueryResult<'array'> = objects;
    // @ts-expect-error the declared mode is closed to unknown strings
    await client.query('VALUES 1', undefined, { rowMode: 'invalid' });
    void [array, nested, dynamic, wrong];
  }
  for (const stmt of statements) {
    const arrays: QueryResult<'array'> = await stmt.execute([], { rowMode: 'array' });
    const batch: QueryResult<'array'> = await stmt.executeBatch([[1]], { rowMode: 'array' });
    const objects: QueryResult = await stmt.execute();
    // @ts-expect-error invalid modes must be caught for batches too
    await stmt.executeBatch([], { rowMode: 'invalid' });
    void [arrays, batch, objects];
  }
  for (const pool of clients.filter((p): p is Pool | CompatPool => p instanceof Pool || p instanceof CompatPool)) {
    pool.query('VALUES 1', [], { rowMode: 'array' }, (error, result) => {
      const row: any[] = result.rows[0];
      const code: Db2Error['driverCode'] = error?.driverCode;
      void [row, code];
    });
    pool.query('VALUES 1', [], (error, result) => { void [error, result]; });
  }
}
void types;
