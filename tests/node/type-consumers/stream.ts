import type { Database, Db2Error, QueryRow, RowMode, RowStream, SqlQuery } from 'db2-node';

declare const database: Database;
declare const dynamic: SqlQuery;
const objects = database.queryStream('VALUES 1');
const arrays = database.queryStream({ sql: 'VALUES 1', rowMode: 'array' });
const union = database.queryStream(dynamic);

// @ts-expect-error default streams cannot promise array rows
const incorrectDefault: RowStream<'array'> = objects;
// @ts-expect-error optional modes cannot promise array rows
const incorrectDynamic: RowStream<'array'> = union;

export async function consume() {
  for await (const row of arrays) {
    const array: any[] = row;
  }
  for await (const row of objects) {
    const object: QueryRow = row;
    // @ts-expect-error default iteration yields object rows
    const array: any[] = row;
  }
  for await (const row of union) {
    const either: QueryRow<RowMode> = row;
    // @ts-expect-error dynamic iteration must allow object rows
    const array: any[] = row;
  }
  const iterator = arrays[Symbol.asyncIterator]();
  const iterableIterator: AsyncIterableIterator<any[]> = iterator;
  const next = await iterator.next();
  if (!next.done) {
    const array: any[] = next.value;
  }
}

arrays.on('data', row => { const array: any[] = row; });
objects.once('data', row => {
  const object: QueryRow = row;
  // @ts-expect-error default data events yield object rows
  const array: any[] = row;
});
union.on('data', row => {
  const either: QueryRow<RowMode> = row;
  // @ts-expect-error dynamic events must allow object rows
  const array: any[] = row;
});
arrays.on('error', error => { const classified: Db2Error = error; });
arrays.once('end', () => {});
arrays.on('readable', () => {});
arrays.on('custom', (...args) => {});
// @ts-expect-error a known data event cannot accept an unrelated listener type
arrays.on('data', (row: number) => {});

const maybeRow: any[] | null = arrays.read();
const maybeObject: QueryRow | null = objects.read();
// @ts-expect-error default read() also yields object rows
const incorrectRead: any[] | null = objects.read();
const paused: boolean = arrays.pause().isPaused();
const same: typeof arrays = arrays.resume().destroy();
const destroyed: boolean = arrays.destroyed;

// No ambient Node types are available in this consumer.
// @ts-expect-error Node globals must stay absent from this compilation
const nodeVersion = process.version;
