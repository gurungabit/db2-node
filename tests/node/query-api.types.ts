// Compile with tsc --strict --noEmit; these calls are never executed.
import type {
  Client,
  Pool,
  Db2Pool,
  CompatPool,
  IbmDbPool,
  NativeClient,
  NativePool,
  JsClient,
  JsPool,
  Transaction,
  NativeTransaction,
  JsTransaction,
  PreparedStatement,
  NativePreparedStatement,
  JsPreparedStatement,
  Database,
  ODBCStatement,
  QueryOptions,
  QueryResult,
  QueryRow,
  RowMode,
  SqlQuery,
  ODBCResult,
  RowStream,
  Db2Error,
} from '../../crates/db2-napi';

declare const client: Client;
declare const pool: Pool;
declare const db2Pool: Db2Pool;
declare const compatPool: CompatPool;
declare const ibmDbPool: InstanceType<typeof IbmDbPool>;
declare const nativeClient: NativeClient;
declare const nativePool: NativePool;
declare const jsClient: JsClient;
declare const jsPool: JsPool;
declare const transaction: Transaction;
declare const nativeTransaction: NativeTransaction;
declare const jsTransaction: JsTransaction;
declare const preparedStatement: PreparedStatement;
declare const nativePreparedStatement: NativePreparedStatement;
declare const jsPreparedStatement: JsPreparedStatement;
declare const database: Database;
declare const odbcStatement: ODBCStatement;
declare const dynamic: QueryOptions;
declare const optionalArray: { rowMode?: 'array' };
declare const maybeOptions: QueryOptions | undefined;
declare const sqlDynamic: SqlQuery;
// @ts-expect-error array options require the actual runtime discriminant
const missingArrayMode: QueryOptions<'array'> = {};
// @ts-expect-error an optional discriminant cannot guarantee array rows
const optionalArrayMode: QueryOptions<'array'> = optionalArray;

async function types() {
  // Client: check the individual signature so a union of APIs cannot mask a bug.
  const clientObjects: QueryResult = await client.query('VALUES 1');
  const clientArrays: QueryResult<'array'> = await client.query('VALUES 1', null, { rowMode: 'array' });
  const clientUnion: QueryResult<RowMode> = await client.query('VALUES 1', [], dynamic);
  const clientOptional: QueryResult<RowMode> = await client.query('VALUES 1', [], optionalArray);
  const clientMaybe: QueryResult<RowMode> = await client.query('VALUES 1', [], maybeOptions);
  // @ts-expect-error expected return type must not invent an omitted array option
  const clientContext: QueryResult<'array'> = await client.query('VALUES 1');
  // @ts-expect-error explicit undefined still selects object rows
  const clientUndefined: QueryResult<'array'> = await client.query('VALUES 1', [], undefined);
  // @ts-expect-error null options still select object rows
  const clientNull: QueryResult<'array'> = await client.query('VALUES 1', [], null);
  // @ts-expect-error empty options still select object rows
  const clientEmpty: QueryResult<'array'> = await client.query('VALUES 1', [], {});
  // @ts-expect-error a dynamic result cannot guarantee array rows
  const clientDynamic: QueryResult<'array'> = await client.query('VALUES 1', [], dynamic);
  // @ts-expect-error an optional mode cannot guarantee array rows
  const clientOptionalBad: QueryResult<'array'> = await client.query('VALUES 1', [], optionalArray);
  // @ts-expect-error unknown row modes are rejected
  await client.query('VALUES 1', [], { rowMode: 'invalid' });
  // Pool: check the individual signature so a union of APIs cannot mask a bug.
  const poolObjects: QueryResult = await pool.query('VALUES 1');
  const poolArrays: QueryResult<'array'> = await pool.query('VALUES 1', null, { rowMode: 'array' });
  const poolUnion: QueryResult<RowMode> = await pool.query('VALUES 1', [], dynamic);
  const poolOptional: QueryResult<RowMode> = await pool.query('VALUES 1', [], optionalArray);
  const poolMaybe: QueryResult<RowMode> = await pool.query('VALUES 1', [], maybeOptions);
  // @ts-expect-error expected return type must not invent an omitted array option
  const poolContext: QueryResult<'array'> = await pool.query('VALUES 1');
  // @ts-expect-error explicit undefined still selects object rows
  const poolUndefined: QueryResult<'array'> = await pool.query('VALUES 1', [], undefined);
  // @ts-expect-error null options still select object rows
  const poolNull: QueryResult<'array'> = await pool.query('VALUES 1', [], null);
  // @ts-expect-error empty options still select object rows
  const poolEmpty: QueryResult<'array'> = await pool.query('VALUES 1', [], {});
  // @ts-expect-error a dynamic result cannot guarantee array rows
  const poolDynamic: QueryResult<'array'> = await pool.query('VALUES 1', [], dynamic);
  // @ts-expect-error an optional mode cannot guarantee array rows
  const poolOptionalBad: QueryResult<'array'> = await pool.query('VALUES 1', [], optionalArray);
  // @ts-expect-error unknown row modes are rejected
  await pool.query('VALUES 1', [], { rowMode: 'invalid' });
  // Db2Pool: check the individual signature so a union of APIs cannot mask a bug.
  const db2PoolObjects: QueryResult = await db2Pool.query('VALUES 1');
  const db2PoolArrays: QueryResult<'array'> = await db2Pool.query('VALUES 1', null, { rowMode: 'array' });
  const db2PoolUnion: QueryResult<RowMode> = await db2Pool.query('VALUES 1', [], dynamic);
  const db2PoolOptional: QueryResult<RowMode> = await db2Pool.query('VALUES 1', [], optionalArray);
  const db2PoolMaybe: QueryResult<RowMode> = await db2Pool.query('VALUES 1', [], maybeOptions);
  // @ts-expect-error expected return type must not invent an omitted array option
  const db2PoolContext: QueryResult<'array'> = await db2Pool.query('VALUES 1');
  // @ts-expect-error explicit undefined still selects object rows
  const db2PoolUndefined: QueryResult<'array'> = await db2Pool.query('VALUES 1', [], undefined);
  // @ts-expect-error null options still select object rows
  const db2PoolNull: QueryResult<'array'> = await db2Pool.query('VALUES 1', [], null);
  // @ts-expect-error empty options still select object rows
  const db2PoolEmpty: QueryResult<'array'> = await db2Pool.query('VALUES 1', [], {});
  // @ts-expect-error a dynamic result cannot guarantee array rows
  const db2PoolDynamic: QueryResult<'array'> = await db2Pool.query('VALUES 1', [], dynamic);
  // @ts-expect-error an optional mode cannot guarantee array rows
  const db2PoolOptionalBad: QueryResult<'array'> = await db2Pool.query('VALUES 1', [], optionalArray);
  // @ts-expect-error unknown row modes are rejected
  await db2Pool.query('VALUES 1', [], { rowMode: 'invalid' });
  // CompatPool: check the individual signature so a union of APIs cannot mask a bug.
  const compatPoolObjects: QueryResult = await compatPool.query('VALUES 1');
  const compatPoolArrays: QueryResult<'array'> = await compatPool.query('VALUES 1', null, { rowMode: 'array' });
  const compatPoolUnion: QueryResult<RowMode> = await compatPool.query('VALUES 1', [], dynamic);
  const compatPoolOptional: QueryResult<RowMode> = await compatPool.query('VALUES 1', [], optionalArray);
  const compatPoolMaybe: QueryResult<RowMode> = await compatPool.query('VALUES 1', [], maybeOptions);
  // @ts-expect-error expected return type must not invent an omitted array option
  const compatPoolContext: QueryResult<'array'> = await compatPool.query('VALUES 1');
  // @ts-expect-error explicit undefined still selects object rows
  const compatPoolUndefined: QueryResult<'array'> = await compatPool.query('VALUES 1', [], undefined);
  // @ts-expect-error null options still select object rows
  const compatPoolNull: QueryResult<'array'> = await compatPool.query('VALUES 1', [], null);
  // @ts-expect-error empty options still select object rows
  const compatPoolEmpty: QueryResult<'array'> = await compatPool.query('VALUES 1', [], {});
  // @ts-expect-error a dynamic result cannot guarantee array rows
  const compatPoolDynamic: QueryResult<'array'> = await compatPool.query('VALUES 1', [], dynamic);
  // @ts-expect-error an optional mode cannot guarantee array rows
  const compatPoolOptionalBad: QueryResult<'array'> = await compatPool.query('VALUES 1', [], optionalArray);
  // @ts-expect-error unknown row modes are rejected
  await compatPool.query('VALUES 1', [], { rowMode: 'invalid' });
  // IbmDbPool: check the individual signature so a union of APIs cannot mask a bug.
  const ibmDbPoolObjects: QueryResult = await ibmDbPool.query('VALUES 1');
  const ibmDbPoolArrays: QueryResult<'array'> = await ibmDbPool.query('VALUES 1', null, { rowMode: 'array' });
  const ibmDbPoolUnion: QueryResult<RowMode> = await ibmDbPool.query('VALUES 1', [], dynamic);
  const ibmDbPoolOptional: QueryResult<RowMode> = await ibmDbPool.query('VALUES 1', [], optionalArray);
  const ibmDbPoolMaybe: QueryResult<RowMode> = await ibmDbPool.query('VALUES 1', [], maybeOptions);
  // @ts-expect-error expected return type must not invent an omitted array option
  const ibmDbPoolContext: QueryResult<'array'> = await ibmDbPool.query('VALUES 1');
  // @ts-expect-error explicit undefined still selects object rows
  const ibmDbPoolUndefined: QueryResult<'array'> = await ibmDbPool.query('VALUES 1', [], undefined);
  // @ts-expect-error null options still select object rows
  const ibmDbPoolNull: QueryResult<'array'> = await ibmDbPool.query('VALUES 1', [], null);
  // @ts-expect-error empty options still select object rows
  const ibmDbPoolEmpty: QueryResult<'array'> = await ibmDbPool.query('VALUES 1', [], {});
  // @ts-expect-error a dynamic result cannot guarantee array rows
  const ibmDbPoolDynamic: QueryResult<'array'> = await ibmDbPool.query('VALUES 1', [], dynamic);
  // @ts-expect-error an optional mode cannot guarantee array rows
  const ibmDbPoolOptionalBad: QueryResult<'array'> = await ibmDbPool.query('VALUES 1', [], optionalArray);
  // @ts-expect-error unknown row modes are rejected
  await ibmDbPool.query('VALUES 1', [], { rowMode: 'invalid' });
  // NativeClient: check the individual signature so a union of APIs cannot mask a bug.
  const nativeClientObjects: QueryResult = await nativeClient.query('VALUES 1');
  const nativeClientArrays: QueryResult<'array'> = await nativeClient.query('VALUES 1', null, { rowMode: 'array' });
  const nativeClientUnion: QueryResult<RowMode> = await nativeClient.query('VALUES 1', [], dynamic);
  const nativeClientOptional: QueryResult<RowMode> = await nativeClient.query('VALUES 1', [], optionalArray);
  const nativeClientMaybe: QueryResult<RowMode> = await nativeClient.query('VALUES 1', [], maybeOptions);
  // @ts-expect-error expected return type must not invent an omitted array option
  const nativeClientContext: QueryResult<'array'> = await nativeClient.query('VALUES 1');
  // @ts-expect-error explicit undefined still selects object rows
  const nativeClientUndefined: QueryResult<'array'> = await nativeClient.query('VALUES 1', [], undefined);
  // @ts-expect-error null options still select object rows
  const nativeClientNull: QueryResult<'array'> = await nativeClient.query('VALUES 1', [], null);
  // @ts-expect-error empty options still select object rows
  const nativeClientEmpty: QueryResult<'array'> = await nativeClient.query('VALUES 1', [], {});
  // @ts-expect-error a dynamic result cannot guarantee array rows
  const nativeClientDynamic: QueryResult<'array'> = await nativeClient.query('VALUES 1', [], dynamic);
  // @ts-expect-error an optional mode cannot guarantee array rows
  const nativeClientOptionalBad: QueryResult<'array'> = await nativeClient.query('VALUES 1', [], optionalArray);
  // @ts-expect-error unknown row modes are rejected
  await nativeClient.query('VALUES 1', [], { rowMode: 'invalid' });
  // NativePool: check the individual signature so a union of APIs cannot mask a bug.
  const nativePoolObjects: QueryResult = await nativePool.query('VALUES 1');
  const nativePoolArrays: QueryResult<'array'> = await nativePool.query('VALUES 1', null, { rowMode: 'array' });
  const nativePoolUnion: QueryResult<RowMode> = await nativePool.query('VALUES 1', [], dynamic);
  const nativePoolOptional: QueryResult<RowMode> = await nativePool.query('VALUES 1', [], optionalArray);
  const nativePoolMaybe: QueryResult<RowMode> = await nativePool.query('VALUES 1', [], maybeOptions);
  // @ts-expect-error expected return type must not invent an omitted array option
  const nativePoolContext: QueryResult<'array'> = await nativePool.query('VALUES 1');
  // @ts-expect-error explicit undefined still selects object rows
  const nativePoolUndefined: QueryResult<'array'> = await nativePool.query('VALUES 1', [], undefined);
  // @ts-expect-error null options still select object rows
  const nativePoolNull: QueryResult<'array'> = await nativePool.query('VALUES 1', [], null);
  // @ts-expect-error empty options still select object rows
  const nativePoolEmpty: QueryResult<'array'> = await nativePool.query('VALUES 1', [], {});
  // @ts-expect-error a dynamic result cannot guarantee array rows
  const nativePoolDynamic: QueryResult<'array'> = await nativePool.query('VALUES 1', [], dynamic);
  // @ts-expect-error an optional mode cannot guarantee array rows
  const nativePoolOptionalBad: QueryResult<'array'> = await nativePool.query('VALUES 1', [], optionalArray);
  // @ts-expect-error unknown row modes are rejected
  await nativePool.query('VALUES 1', [], { rowMode: 'invalid' });
  // JsClient: check the individual signature so a union of APIs cannot mask a bug.
  const jsClientObjects: QueryResult = await jsClient.query('VALUES 1');
  const jsClientArrays: QueryResult<'array'> = await jsClient.query('VALUES 1', null, { rowMode: 'array' });
  const jsClientUnion: QueryResult<RowMode> = await jsClient.query('VALUES 1', [], dynamic);
  const jsClientOptional: QueryResult<RowMode> = await jsClient.query('VALUES 1', [], optionalArray);
  const jsClientMaybe: QueryResult<RowMode> = await jsClient.query('VALUES 1', [], maybeOptions);
  // @ts-expect-error expected return type must not invent an omitted array option
  const jsClientContext: QueryResult<'array'> = await jsClient.query('VALUES 1');
  // @ts-expect-error explicit undefined still selects object rows
  const jsClientUndefined: QueryResult<'array'> = await jsClient.query('VALUES 1', [], undefined);
  // @ts-expect-error null options still select object rows
  const jsClientNull: QueryResult<'array'> = await jsClient.query('VALUES 1', [], null);
  // @ts-expect-error empty options still select object rows
  const jsClientEmpty: QueryResult<'array'> = await jsClient.query('VALUES 1', [], {});
  // @ts-expect-error a dynamic result cannot guarantee array rows
  const jsClientDynamic: QueryResult<'array'> = await jsClient.query('VALUES 1', [], dynamic);
  // @ts-expect-error an optional mode cannot guarantee array rows
  const jsClientOptionalBad: QueryResult<'array'> = await jsClient.query('VALUES 1', [], optionalArray);
  // @ts-expect-error unknown row modes are rejected
  await jsClient.query('VALUES 1', [], { rowMode: 'invalid' });
  // JsPool: check the individual signature so a union of APIs cannot mask a bug.
  const jsPoolObjects: QueryResult = await jsPool.query('VALUES 1');
  const jsPoolArrays: QueryResult<'array'> = await jsPool.query('VALUES 1', null, { rowMode: 'array' });
  const jsPoolUnion: QueryResult<RowMode> = await jsPool.query('VALUES 1', [], dynamic);
  const jsPoolOptional: QueryResult<RowMode> = await jsPool.query('VALUES 1', [], optionalArray);
  const jsPoolMaybe: QueryResult<RowMode> = await jsPool.query('VALUES 1', [], maybeOptions);
  // @ts-expect-error expected return type must not invent an omitted array option
  const jsPoolContext: QueryResult<'array'> = await jsPool.query('VALUES 1');
  // @ts-expect-error explicit undefined still selects object rows
  const jsPoolUndefined: QueryResult<'array'> = await jsPool.query('VALUES 1', [], undefined);
  // @ts-expect-error null options still select object rows
  const jsPoolNull: QueryResult<'array'> = await jsPool.query('VALUES 1', [], null);
  // @ts-expect-error empty options still select object rows
  const jsPoolEmpty: QueryResult<'array'> = await jsPool.query('VALUES 1', [], {});
  // @ts-expect-error a dynamic result cannot guarantee array rows
  const jsPoolDynamic: QueryResult<'array'> = await jsPool.query('VALUES 1', [], dynamic);
  // @ts-expect-error an optional mode cannot guarantee array rows
  const jsPoolOptionalBad: QueryResult<'array'> = await jsPool.query('VALUES 1', [], optionalArray);
  // @ts-expect-error unknown row modes are rejected
  await jsPool.query('VALUES 1', [], { rowMode: 'invalid' });
  // Transaction: check the individual signature so a union of APIs cannot mask a bug.
  const transactionObjects: QueryResult = await transaction.query('VALUES 1');
  const transactionArrays: QueryResult<'array'> = await transaction.query('VALUES 1', null, { rowMode: 'array' });
  const transactionUnion: QueryResult<RowMode> = await transaction.query('VALUES 1', [], dynamic);
  const transactionOptional: QueryResult<RowMode> = await transaction.query('VALUES 1', [], optionalArray);
  const transactionMaybe: QueryResult<RowMode> = await transaction.query('VALUES 1', [], maybeOptions);
  // @ts-expect-error expected return type must not invent an omitted array option
  const transactionContext: QueryResult<'array'> = await transaction.query('VALUES 1');
  // @ts-expect-error explicit undefined still selects object rows
  const transactionUndefined: QueryResult<'array'> = await transaction.query('VALUES 1', [], undefined);
  // @ts-expect-error null options still select object rows
  const transactionNull: QueryResult<'array'> = await transaction.query('VALUES 1', [], null);
  // @ts-expect-error empty options still select object rows
  const transactionEmpty: QueryResult<'array'> = await transaction.query('VALUES 1', [], {});
  // @ts-expect-error a dynamic result cannot guarantee array rows
  const transactionDynamic: QueryResult<'array'> = await transaction.query('VALUES 1', [], dynamic);
  // @ts-expect-error an optional mode cannot guarantee array rows
  const transactionOptionalBad: QueryResult<'array'> = await transaction.query('VALUES 1', [], optionalArray);
  // @ts-expect-error unknown row modes are rejected
  await transaction.query('VALUES 1', [], { rowMode: 'invalid' });
  // NativeTransaction: check the individual signature so a union of APIs cannot mask a bug.
  const nativeTransactionObjects: QueryResult = await nativeTransaction.query('VALUES 1');
  const nativeTransactionArrays: QueryResult<'array'> = await nativeTransaction.query('VALUES 1', null, { rowMode: 'array' });
  const nativeTransactionUnion: QueryResult<RowMode> = await nativeTransaction.query('VALUES 1', [], dynamic);
  const nativeTransactionOptional: QueryResult<RowMode> = await nativeTransaction.query('VALUES 1', [], optionalArray);
  const nativeTransactionMaybe: QueryResult<RowMode> = await nativeTransaction.query('VALUES 1', [], maybeOptions);
  // @ts-expect-error expected return type must not invent an omitted array option
  const nativeTransactionContext: QueryResult<'array'> = await nativeTransaction.query('VALUES 1');
  // @ts-expect-error explicit undefined still selects object rows
  const nativeTransactionUndefined: QueryResult<'array'> = await nativeTransaction.query('VALUES 1', [], undefined);
  // @ts-expect-error null options still select object rows
  const nativeTransactionNull: QueryResult<'array'> = await nativeTransaction.query('VALUES 1', [], null);
  // @ts-expect-error empty options still select object rows
  const nativeTransactionEmpty: QueryResult<'array'> = await nativeTransaction.query('VALUES 1', [], {});
  // @ts-expect-error a dynamic result cannot guarantee array rows
  const nativeTransactionDynamic: QueryResult<'array'> = await nativeTransaction.query('VALUES 1', [], dynamic);
  // @ts-expect-error an optional mode cannot guarantee array rows
  const nativeTransactionOptionalBad: QueryResult<'array'> = await nativeTransaction.query('VALUES 1', [], optionalArray);
  // @ts-expect-error unknown row modes are rejected
  await nativeTransaction.query('VALUES 1', [], { rowMode: 'invalid' });
  // JsTransaction: check the individual signature so a union of APIs cannot mask a bug.
  const jsTransactionObjects: QueryResult = await jsTransaction.query('VALUES 1');
  const jsTransactionArrays: QueryResult<'array'> = await jsTransaction.query('VALUES 1', null, { rowMode: 'array' });
  const jsTransactionUnion: QueryResult<RowMode> = await jsTransaction.query('VALUES 1', [], dynamic);
  const jsTransactionOptional: QueryResult<RowMode> = await jsTransaction.query('VALUES 1', [], optionalArray);
  const jsTransactionMaybe: QueryResult<RowMode> = await jsTransaction.query('VALUES 1', [], maybeOptions);
  // @ts-expect-error expected return type must not invent an omitted array option
  const jsTransactionContext: QueryResult<'array'> = await jsTransaction.query('VALUES 1');
  // @ts-expect-error explicit undefined still selects object rows
  const jsTransactionUndefined: QueryResult<'array'> = await jsTransaction.query('VALUES 1', [], undefined);
  // @ts-expect-error null options still select object rows
  const jsTransactionNull: QueryResult<'array'> = await jsTransaction.query('VALUES 1', [], null);
  // @ts-expect-error empty options still select object rows
  const jsTransactionEmpty: QueryResult<'array'> = await jsTransaction.query('VALUES 1', [], {});
  // @ts-expect-error a dynamic result cannot guarantee array rows
  const jsTransactionDynamic: QueryResult<'array'> = await jsTransaction.query('VALUES 1', [], dynamic);
  // @ts-expect-error an optional mode cannot guarantee array rows
  const jsTransactionOptionalBad: QueryResult<'array'> = await jsTransaction.query('VALUES 1', [], optionalArray);
  // @ts-expect-error unknown row modes are rejected
  await jsTransaction.query('VALUES 1', [], { rowMode: 'invalid' });
  const preparedStatementexecuteObjects: QueryResult = await preparedStatement.execute([]);
  const preparedStatementexecuteArrays: QueryResult<'array'> = await preparedStatement.execute([], { rowMode: 'array' });
  const preparedStatementexecuteUnion: QueryResult<RowMode> = await preparedStatement.execute([], optionalArray);
  // @ts-expect-error contextual return inference cannot change the runtime row mode
  const preparedStatementexecuteContext: QueryResult<'array'> = await preparedStatement.execute([]);
  // @ts-expect-error empty options cannot promise array rows
  const preparedStatementexecuteEmpty: QueryResult<'array'> = await preparedStatement.execute([], {});
  // @ts-expect-error optional mode cannot promise array rows
  const preparedStatementexecuteOptional: QueryResult<'array'> = await preparedStatement.execute([], optionalArray);
  // @ts-expect-error unknown modes are rejected
  await preparedStatement.execute([], { rowMode: 'invalid' });
  const preparedStatementexecuteBatchObjects: QueryResult = await preparedStatement.executeBatch([[1]]);
  const preparedStatementexecuteBatchArrays: QueryResult<'array'> = await preparedStatement.executeBatch([[1]], { rowMode: 'array' });
  const preparedStatementexecuteBatchUnion: QueryResult<RowMode> = await preparedStatement.executeBatch([[1]], optionalArray);
  // @ts-expect-error contextual return inference cannot change the runtime row mode
  const preparedStatementexecuteBatchContext: QueryResult<'array'> = await preparedStatement.executeBatch([[1]]);
  // @ts-expect-error empty options cannot promise array rows
  const preparedStatementexecuteBatchEmpty: QueryResult<'array'> = await preparedStatement.executeBatch([[1]], {});
  // @ts-expect-error optional mode cannot promise array rows
  const preparedStatementexecuteBatchOptional: QueryResult<'array'> = await preparedStatement.executeBatch([[1]], optionalArray);
  // @ts-expect-error unknown modes are rejected
  await preparedStatement.executeBatch([[1]], { rowMode: 'invalid' });
  const nativePreparedStatementexecuteObjects: QueryResult = await nativePreparedStatement.execute([]);
  const nativePreparedStatementexecuteArrays: QueryResult<'array'> = await nativePreparedStatement.execute([], { rowMode: 'array' });
  const nativePreparedStatementexecuteUnion: QueryResult<RowMode> = await nativePreparedStatement.execute([], optionalArray);
  // @ts-expect-error contextual return inference cannot change the runtime row mode
  const nativePreparedStatementexecuteContext: QueryResult<'array'> = await nativePreparedStatement.execute([]);
  // @ts-expect-error empty options cannot promise array rows
  const nativePreparedStatementexecuteEmpty: QueryResult<'array'> = await nativePreparedStatement.execute([], {});
  // @ts-expect-error optional mode cannot promise array rows
  const nativePreparedStatementexecuteOptional: QueryResult<'array'> = await nativePreparedStatement.execute([], optionalArray);
  // @ts-expect-error unknown modes are rejected
  await nativePreparedStatement.execute([], { rowMode: 'invalid' });
  const nativePreparedStatementexecuteBatchObjects: QueryResult = await nativePreparedStatement.executeBatch([[1]]);
  const nativePreparedStatementexecuteBatchArrays: QueryResult<'array'> = await nativePreparedStatement.executeBatch([[1]], { rowMode: 'array' });
  const nativePreparedStatementexecuteBatchUnion: QueryResult<RowMode> = await nativePreparedStatement.executeBatch([[1]], optionalArray);
  // @ts-expect-error contextual return inference cannot change the runtime row mode
  const nativePreparedStatementexecuteBatchContext: QueryResult<'array'> = await nativePreparedStatement.executeBatch([[1]]);
  // @ts-expect-error empty options cannot promise array rows
  const nativePreparedStatementexecuteBatchEmpty: QueryResult<'array'> = await nativePreparedStatement.executeBatch([[1]], {});
  // @ts-expect-error optional mode cannot promise array rows
  const nativePreparedStatementexecuteBatchOptional: QueryResult<'array'> = await nativePreparedStatement.executeBatch([[1]], optionalArray);
  // @ts-expect-error unknown modes are rejected
  await nativePreparedStatement.executeBatch([[1]], { rowMode: 'invalid' });
  const jsPreparedStatementexecuteObjects: QueryResult = await jsPreparedStatement.execute([]);
  const jsPreparedStatementexecuteArrays: QueryResult<'array'> = await jsPreparedStatement.execute([], { rowMode: 'array' });
  const jsPreparedStatementexecuteUnion: QueryResult<RowMode> = await jsPreparedStatement.execute([], optionalArray);
  // @ts-expect-error contextual return inference cannot change the runtime row mode
  const jsPreparedStatementexecuteContext: QueryResult<'array'> = await jsPreparedStatement.execute([]);
  // @ts-expect-error empty options cannot promise array rows
  const jsPreparedStatementexecuteEmpty: QueryResult<'array'> = await jsPreparedStatement.execute([], {});
  // @ts-expect-error optional mode cannot promise array rows
  const jsPreparedStatementexecuteOptional: QueryResult<'array'> = await jsPreparedStatement.execute([], optionalArray);
  // @ts-expect-error unknown modes are rejected
  await jsPreparedStatement.execute([], { rowMode: 'invalid' });
  const jsPreparedStatementexecuteBatchObjects: QueryResult = await jsPreparedStatement.executeBatch([[1]]);
  const jsPreparedStatementexecuteBatchArrays: QueryResult<'array'> = await jsPreparedStatement.executeBatch([[1]], { rowMode: 'array' });
  const jsPreparedStatementexecuteBatchUnion: QueryResult<RowMode> = await jsPreparedStatement.executeBatch([[1]], optionalArray);
  // @ts-expect-error contextual return inference cannot change the runtime row mode
  const jsPreparedStatementexecuteBatchContext: QueryResult<'array'> = await jsPreparedStatement.executeBatch([[1]]);
  // @ts-expect-error empty options cannot promise array rows
  const jsPreparedStatementexecuteBatchEmpty: QueryResult<'array'> = await jsPreparedStatement.executeBatch([[1]], {});
  // @ts-expect-error optional mode cannot promise array rows
  const jsPreparedStatementexecuteBatchOptional: QueryResult<'array'> = await jsPreparedStatement.executeBatch([[1]], optionalArray);
  // @ts-expect-error unknown modes are rejected
  await jsPreparedStatement.executeBatch([[1]], { rowMode: 'invalid' });
  pool.query('VALUES 1', [], { rowMode: 'array' }, (error, result) => {
    const row: any[] = result.rows[0];
    const nested: any[] = result.resultSets[0].rows[0];
    const code: Db2Error['driverCode'] = error?.driverCode;
  });
  pool.query('VALUES 1', [], (error, result) => {
    const object: QueryResult = result;
    // @ts-expect-error callbacks without options receive object rows
    const array: QueryResult<'array'> = result;
  });
  pool.query('VALUES 1', (error, result) => {
    // @ts-expect-error params-free callbacks also receive object rows
    const array: QueryResult<'array'> = result;
  });
  pool.query('VALUES 1', [], optionalArray, (error, result) => {
    const union: QueryResult<RowMode> = result;
    // @ts-expect-error dynamic callbacks must allow both row shapes
    const array: QueryResult<'array'> = result;
  });
  compatPool.query('VALUES 1', [], { rowMode: 'array' }, (error, result) => {
    const row: any[] = result.rows[0];
    const nested: any[] = result.resultSets[0].rows[0];
    const code: Db2Error['driverCode'] = error?.driverCode;
  });
  compatPool.query('VALUES 1', [], (error, result) => {
    const object: QueryResult = result;
    // @ts-expect-error callbacks without options receive object rows
    const array: QueryResult<'array'> = result;
  });
  compatPool.query('VALUES 1', (error, result) => {
    // @ts-expect-error params-free callbacks also receive object rows
    const array: QueryResult<'array'> = result;
  });
  compatPool.query('VALUES 1', [], optionalArray, (error, result) => {
    const union: QueryResult<RowMode> = result;
    // @ts-expect-error dynamic callbacks must allow both row shapes
    const array: QueryResult<'array'> = result;
  });
  ibmDbPool.query('VALUES 1', [], { rowMode: 'array' }, (error, result) => {
    const row: any[] = result.rows[0];
    const nested: any[] = result.resultSets[0].rows[0];
    const code: Db2Error['driverCode'] = error?.driverCode;
  });
  ibmDbPool.query('VALUES 1', [], (error, result) => {
    const object: QueryResult = result;
    // @ts-expect-error callbacks without options receive object rows
    const array: QueryResult<'array'> = result;
  });
  ibmDbPool.query('VALUES 1', (error, result) => {
    // @ts-expect-error params-free callbacks also receive object rows
    const array: QueryResult<'array'> = result;
  });
  ibmDbPool.query('VALUES 1', [], optionalArray, (error, result) => {
    const union: QueryResult<RowMode> = result;
    // @ts-expect-error dynamic callbacks must allow both row shapes
    const array: QueryResult<'array'> = result;
  });
  const databaseObjects: QueryRow[] = await database.query('VALUES 1');
  const databaseArrays: any[][] = await database.query({ sql: 'VALUES 1', rowMode: 'array' });
  const databaseDynamic: QueryRow<RowMode>[] = await database.query(sqlDynamic);
  // @ts-expect-error SQL strings default to object rows
  const databaseContext: any[][] = await database.query('VALUES 1');
  // @ts-expect-error SQL objects without a mode default to object rows
  const databaseEmpty: any[][] = await database.query({ sql: 'VALUES 1' });
  // @ts-expect-error dynamic SQL objects cannot guarantee array rows
  const databaseDynamicBad: any[][] = await database.query(sqlDynamic);
  database.query({ sql: 'VALUES 1', rowMode: 'array' }, (error, rows) => {
    const array: any[][] = rows;
  });
  database.query('VALUES 1', (error, rows) => {
    // @ts-expect-error default callbacks receive object rows
    const array: any[][] = rows;
  });
  database.query(sqlDynamic, [], (error, rows) => {
    // @ts-expect-error dynamic callbacks must allow object rows too
    const array: any[][] = rows;
  });
  const result: ODBCResult<'array'> = await database.queryResult({ sql: 'VALUES 1', rowMode: 'array' });
  const fetched: any[] | false = await result.fetch();
  const fetchedAll: any[][] = await result.fetchAll();
  const nested: QueryResult<'array'> = result.resultSets[0];
  // @ts-expect-error queryResult also defaults to object rows
  const defaultResult: ODBCResult<'array'> = await database.queryResult('VALUES 1');
  // @ts-expect-error optional SQL modes produce a union result
  const optionalResult: ODBCResult<'array'> = await database.queryResult(sqlDynamic);
  database.queryResult('VALUES 1', (error, result) => {
    // @ts-expect-error result callbacks default to object rows
    const array: ODBCResult<'array'> = result;
  });
  database.queryResult({ sql: 'VALUES 1', rowMode: 'array' }, [], (error, result) => {
    const array: ODBCResult<'array'> = result;
  });
  const arrayStream: RowStream<'array'> = database.queryStream({ sql: 'VALUES 1', rowMode: 'array' });
  // @ts-expect-error default streams yield object rows
  const defaultStream: RowStream<'array'> = database.queryStream('VALUES 1');
  // @ts-expect-error dynamic streams yield either row shape
  const dynamicStream: RowStream<'array'> = database.queryStream(sqlDynamic);
  // @ts-expect-error ODBC prepared statements have no array option
  const odbcContext: ODBCResult<'array'> = await odbcStatement.execute();
}
void types;
