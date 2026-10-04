import { Readable, Writable } from 'node:stream';
import { pipeline } from 'node:stream/promises';
import type { Database, RowStream } from 'db2-node';

declare const database: Database;
const rows = database.queryStream({ sql: 'VALUES 1', rowMode: 'array' });
const destination = new Writable({ objectMode: true, write(row, encoding, done) { done(); } });
const piped: Writable = rows.pipe(destination, { end: false });
const defaultEnd: Writable = rows.pipe(destination, { end: undefined });
const unpiped: RowStream<'array'> = rows.unpipe(destination);

// Typed Node consumers can use standard Readable helpers and Writable targets.
const iteration: Pick<Readable, typeof Symbol.asyncIterator> = rows;
const paused: ReturnType<Readable['isPaused']> = rows.pause().isPaused();
const resumed: RowStream<'array'> = rows.resume();
const nativeReadable: Readable = Readable.from(rows);
const pipelined: Promise<void> = pipeline(rows, destination);
rows.on('data', row => { const array: any[] = row; });
rows.on('error', error => { const code: string | undefined = error.driverCode; });
rows.once('end', () => { destination.end(); });
const iterator: AsyncIterableIterator<any[]> = rows[Symbol.asyncIterator]();
