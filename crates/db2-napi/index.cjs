const { Readable } = require('node:stream')
const native = require('./index.js')

const {
  JsClient,
  JsPool,
  JsPreparedStatement,
  JsTransaction,
} = native

let debugEnabled = false

function callbackOrPromise(work, callback, mapValue) {
  const promise = Promise.resolve()
    .then(work)
    .then((value) => (mapValue ? mapValue(value) : value))

  if (typeof callback === 'function') {
    promise.then(
      (value) => callback(null, value),
      (error) => callback(error)
    )
    return undefined
  }

  return promise
}

function callbackOrPromiseMany(work, callback, mapValues) {
  const promise = Promise.resolve()
    .then(work)
    .then((value) => (mapValues ? mapValues(value) : [value]))

  if (typeof callback === 'function') {
    promise.then(
      (values) => callback(null, ...values),
      (error) => callback(error)
    )
    return undefined
  }

  return promise.then((values) => values[0])
}

function unsupportedSync(name) {
  throw new Error(
    `${name} is not supported by @gurungabit/db2-node's ibm_db compatibility layer; use the async/callback API`
  )
}

function normalizeKey(key) {
  return String(key || '')
    .trim()
    .replace(/[_\s-]/g, '')
    .toUpperCase()
}

function parseBool(value) {
  if (typeof value === 'boolean') return value
  const normalized = String(value || '').trim().toLowerCase()
  return normalized === '1' || normalized === 'true' || normalized === 'yes' || normalized === 'on'
}

function parseConnectionString(connectionString, options = {}) {
  if (connectionString && typeof connectionString === 'object') {
    return { ...connectionString }
  }

  if (typeof connectionString !== 'string') {
    throw new TypeError('connectionString must be a string or connection config object')
  }

  const parts = {}
  for (const segment of connectionString.split(';')) {
    if (!segment.trim()) continue
    const eq = segment.indexOf('=')
    if (eq < 0) continue
    const key = normalizeKey(segment.slice(0, eq))
    const value = segment.slice(eq + 1).trim()
    parts[key] = value
  }

  const config = {
    host: parts.HOSTNAME || parts.HOST || parts.SERVER || '',
    database: parts.DATABASE || parts.DB || parts.DSN || '',
    user: parts.UID || parts.USERID || parts.USER || '',
    password: parts.PWD || parts.PASSWORD || '',
  }

  if (config.host.includes(':') && !parts.PORT) {
    const [host, port] = config.host.split(':')
    config.host = host
    config.port = Number(port)
  } else if (parts.PORT) {
    config.port = Number(parts.PORT)
  }

  const security = (parts.SECURITY || parts.SSL || '').toLowerCase()
  if (security === 'ssl' || parseBool(parts.SSL)) {
    config.ssl = true
  }

  if (parts.SSLSERVERCERTIFICATE) {
    config.ssl = true
    config.caCert = parts.SSLSERVERCERTIFICATE
  }

  const schema = parts.CURRENTSCHEMA
  if (schema) config.currentSchema = schema

  if (parts.CONNECTTIMEOUT) {
    config.connectTimeout = Number(parts.CONNECTTIMEOUT) * 1000
  }
  if (parts.QUERYTIMEOUT) {
    config.queryTimeout = Number(parts.QUERYTIMEOUT) * 1000
  }

  if (options && typeof options === 'object') {
    if (options.connectTimeout != null) {
      config.connectTimeout = Number(options.connectTimeout) * 1000
    }
    if (options.queryTimeout != null) {
      config.queryTimeout = Number(options.queryTimeout) * 1000
    }
    if (options.currentSchema != null) config.currentSchema = options.currentSchema
    if (options.fetchSize != null) config.fetchSize = Number(options.fetchSize)
    if (options.ssl != null) config.ssl = parseBool(options.ssl)
    if (options.rejectUnauthorized != null) {
      config.rejectUnauthorized = parseBool(options.rejectUnauthorized)
    }
  }

  if (!config.host) {
    throw new Error(
      'HOSTNAME/HOST is required by @gurungabit/db2-node; DSN-only local ODBC strings are not supported'
    )
  }
  if (!config.database) throw new Error('DATABASE or DSN is required')
  if (!config.user) throw new Error('UID or USER is required')

  return config
}

function normalizeQueryArgs(sqlQuery, bindingParameters, callback) {
  let sql = sqlQuery
  let params = bindingParameters
  let cb = callback
  let noResults = false

  if (typeof params === 'function') {
    cb = params
    params = undefined
  }

  if (sqlQuery && typeof sqlQuery === 'object') {
    sql = sqlQuery.sql
    params = sqlQuery.params || params
    noResults = Boolean(sqlQuery.noResults)
  }

  if (typeof sql !== 'string' || !sql.trim()) {
    throw new TypeError('sqlQuery must be a SQL string or an object with a sql field')
  }

  return { sql, params: Array.isArray(params) ? params : undefined, callback: cb, noResults }
}

function sqlcaFromResult(result) {
  return {
    rowCount: result ? result.rowCount : 0,
    columns: result ? result.columns || [] : [],
    diagnostics: result ? result.diagnostics || [] : [],
  }
}

class ODBCResult {
  constructor(result) {
    this.rows = Array.isArray(result && result.rows) ? result.rows : []
    this.columns = Array.isArray(result && result.columns) ? result.columns : []
    this.rowCount = result && typeof result.rowCount === 'number' ? result.rowCount : this.rows.length
    this.diagnostics = Array.isArray(result && result.diagnostics) ? result.diagnostics : []
    this._offset = 0
    this._closed = false
  }

  fetch(option, callback) {
    if (typeof option === 'function') callback = option
    return callbackOrPromise(() => this.fetchSync(), callback)
  }

  fetchSync() {
    if (this._closed) return false
    if (this._offset >= this.rows.length) return false
    return this.rows[this._offset++]
  }

  fetchAll(option, callback) {
    if (typeof option === 'function') callback = option
    return callbackOrPromise(() => this.fetchAllSync(), callback)
  }

  fetchAllSync() {
    if (this._closed) return []
    const rows = this.rows.slice(this._offset)
    this._offset = this.rows.length
    return rows
  }

  fetchN(count, option, callback) {
    if (typeof option === 'function') callback = option
    return callbackOrPromise(() => this.fetchNSync(count), callback)
  }

  fetchNSync(count) {
    if (this._closed) return []
    const end = Math.min(this._offset + Number(count || 0), this.rows.length)
    const rows = this.rows.slice(this._offset, end)
    this._offset = end
    return rows
  }

  getColumnNamesSync() {
    return this.columns.map((column) => column.name)
  }

  getColumnMetadataSync() {
    return this.columns.map((column, index) => ({
      SQL_DESC_NAME: column.name,
      SQL_DESC_TYPE_NAME: column.db2TypeName || column.typeName,
      SQL_DESC_NULLABLE: column.nullable ? 1 : 0,
      SQL_DESC_PRECISION: column.precision,
      SQL_DESC_SCALE: column.scale,
      index: index + 1,
      name: column.name,
      typeName: column.db2TypeName || column.typeName,
      nullable: column.nullable,
      precision: column.precision,
      scale: column.scale,
    }))
  }

  getSQLErrorSync() {
    return null
  }

  close(callback) {
    return callbackOrPromise(() => this.closeSync(), callback)
  }

  closeSync() {
    this._closed = true
    return true
  }
}

class ODBCStatement {
  constructor(stmt) {
    this._stmt = stmt
    this._boundParams = undefined
    this._closed = false
  }

  bind(bindingParameters, callback) {
    return callbackOrPromise(() => {
      this.bindSync(bindingParameters)
    }, callback)
  }

  bindSync(bindingParameters) {
    this._boundParams = Array.isArray(bindingParameters) ? bindingParameters : []
    return true
  }

  execute(bindingParameters, callback) {
    if (typeof bindingParameters === 'function') {
      callback = bindingParameters
      bindingParameters = undefined
    }
    const params = Array.isArray(bindingParameters) ? bindingParameters : this._boundParams
    return callbackOrPromiseMany(
      async () => new ODBCResult(await this._stmt.execute(params || null)),
      callback,
      (result) => [result, undefined]
    )
  }

  executeSync() {
    return unsupportedSync('ODBCStatement.executeSync')
  }

  executeNonQuery(bindingParameters, callback) {
    if (typeof bindingParameters === 'function') {
      callback = bindingParameters
      bindingParameters = undefined
    }
    const params = Array.isArray(bindingParameters) ? bindingParameters : this._boundParams
    return callbackOrPromise(
      async () => {
        const result = await this._stmt.execute(params || null)
        return result.rowCount || 0
      },
      callback
    )
  }

  executeNonQuerySync() {
    return unsupportedSync('ODBCStatement.executeNonQuerySync')
  }

  close(closeOption, callback) {
    if (typeof closeOption === 'function') callback = closeOption
    return callbackOrPromise(async () => {
      if (!this._closed) {
        this._closed = true
        await this._stmt.close()
      }
    }, callback)
  }

  closeSync() {
    if (!this._closed) {
      this._closed = true
      this._stmt.close().catch(() => {})
    }
    return true
  }
}

class Database {
  constructor(client, onClose, pool) {
    this._client = client
    this._onClose = onClose
    this._pool = pool
    this._closed = false
    this._transaction = null
  }

  _executor() {
    return this._transaction || this._client
  }

  async _ensureClient() {
    if (this._client) return this._client
    if (!this._pool) throw new Error('Database is not connected')
    this._client = await this._pool.acquire()
    return this._client
  }

  query(sqlQuery, bindingParameters, callback) {
    const args = normalizeQueryArgs(sqlQuery, bindingParameters, callback)
    return callbackOrPromiseMany(
      async () => {
        const executor = this._executor()
        const result = executor
          ? await executor.query(args.sql, args.params || null)
          : await this._pool.query(args.sql, args.params || null)
        const rows = args.noResults ? [] : result.rows
        return { rows, sqlca: sqlcaFromResult(result) }
      },
      args.callback,
      ({ rows, sqlca }) => [rows, sqlca]
    )
  }

  querySync() {
    return unsupportedSync('Database.querySync')
  }

  queryResult(sqlQuery, bindingParameters, callback) {
    const args = normalizeQueryArgs(sqlQuery, bindingParameters, callback)
    return callbackOrPromiseMany(
      async () => {
        const executor = this._executor()
        const result = executor
          ? await executor.query(args.sql, args.params || null)
          : await this._pool.query(args.sql, args.params || null)
        return new ODBCResult(result)
      },
      args.callback,
      (result) => [result, undefined]
    )
  }

  queryResultSync() {
    return unsupportedSync('Database.queryResultSync')
  }

  queryStream(sqlQuery, bindingParameters) {
    const stream = new Readable({ objectMode: true, read() {} })
    this.query(sqlQuery, bindingParameters)
      .then((rows) => {
        for (const row of rows) stream.push(row)
        stream.push(null)
      })
      .catch((error) => stream.destroy(error))
    return stream
  }

  prepare(sql, callback) {
    return callbackOrPromise(
      async () => {
        const client = await this._ensureClient()
        return new ODBCStatement(await client.prepare(sql))
      },
      callback
    )
  }

  prepareSync() {
    return unsupportedSync('Database.prepareSync')
  }

  beginTransaction(callback) {
    return callbackOrPromise(async () => {
      const client = await this._ensureClient()
      this._transaction = await client.beginTransaction()
    }, callback)
  }

  beginTransactionSync() {
    return unsupportedSync('Database.beginTransactionSync')
  }

  commitTransaction(callback) {
    return callbackOrPromise(async () => {
      if (this._transaction) {
        await this._transaction.commit()
        this._transaction = null
      }
    }, callback)
  }

  commitTransactionSync() {
    return unsupportedSync('Database.commitTransactionSync')
  }

  rollbackTransaction(callback) {
    return callbackOrPromise(async () => {
      if (this._transaction) {
        await this._transaction.rollback()
        this._transaction = null
      }
    }, callback)
  }

  rollbackTransactionSync() {
    return unsupportedSync('Database.rollbackTransactionSync')
  }

  close(callback) {
    return callbackOrPromise(async () => {
      if (this._closed) return
      this._closed = true
      if (this._transaction) {
        await this._transaction.rollback().catch(() => {})
        this._transaction = null
      }
      if (this._onClose) {
        await this._onClose(this._client)
      } else if (this._client) {
        await this._client.close()
      }
      this._client = null
    }, callback)
  }

  closeSync() {
    this.close().catch(() => {})
    return true
  }

  setIsolationLevel() {
    return true
  }

  setAttr(attributeName, value, callback) {
    if (typeof value === 'function') callback = value
    return callbackOrPromise(() => true, callback)
  }

  setAttrSync() {
    return true
  }

  getInfo(infoType, infoLength, callback) {
    if (typeof infoLength === 'function') callback = infoLength
    return callbackOrPromise(async () => {
      const client = await this._ensureClient()
      return client.serverInfo()
    }, callback)
  }

  getInfoSync() {
    return unsupportedSync('Database.getInfoSync')
  }
}

function open(connectionString, options, callback) {
  if (typeof options === 'function') {
    callback = options
    options = undefined
  }

  return callbackOrPromise(
    async () => {
      const config = parseConnectionString(connectionString, options)
      const pool = new JsPool({ ...config, maxConnections: 1 })
      const validationClient = await pool.acquire()
      await pool.release(validationClient)
      return new Database(null, async (releasedClient) => {
        if (releasedClient) await pool.release(releasedClient)
        await pool.close()
      }, pool)
    },
    callback
  )
}

function openSync() {
  return unsupportedSync('ibmdb.openSync')
}

class Pool {
  constructor(config) {
    this._maxPoolSize = 10
    this._connections = new Set()
    this._native = null
    this._nativeMode = Boolean(config)

    if (config) {
      this._native = new JsPool(config)
    }
  }

  _requireNative() {
    if (!this._native) {
      throw new Error('Pool is not initialized; pass a config to new Pool(config) or call init/initAsync/open first')
    }
    return this._native
  }

  connect(callback) {
    return callbackOrPromise(() => this._requireNative().connect(), callback)
  }

  warmup(callback) {
    return callbackOrPromise(() => this._requireNative().warmup(), callback)
  }

  query(sql, params, callback) {
    if (typeof params === 'function') {
      callback = params
      params = undefined
    }
    return callbackOrPromise(() => this._requireNative().query(sql, params || null), callback)
  }

  acquire(callback) {
    return callbackOrPromise(() => this._requireNative().acquire(), callback)
  }

  release(client, callback) {
    return callbackOrPromise(() => this._requireNative().release(client), callback)
  }

  idleCount(callback) {
    return callbackOrPromise(() => this._requireNative().idleCount(), callback)
  }

  activeCount(callback) {
    return callbackOrPromise(() => this._requireNative().activeCount(), callback)
  }

  totalCount(callback) {
    return callbackOrPromise(() => this._requireNative().totalCount(), callback)
  }

  maxConnections(callback) {
    if (typeof callback === 'function') {
      return callbackOrPromise(() => this._requireNative().maxConnections(), callback)
    }
    return this._requireNative().maxConnections()
  }

  setMaxPoolSize(size) {
    this._maxPoolSize = Number(size) || this._maxPoolSize
    return true
  }

  init(size, connectionString) {
    this._maxPoolSize = Number(size) || this._maxPoolSize
    const config = parseConnectionString(connectionString)
    config.minConnections = this._maxPoolSize
    config.maxConnections = this._maxPoolSize
    this._native = new JsPool(config)
    this._nativeMode = false
    return true
  }

  initAsync(size, connectionString, callback) {
    return callbackOrPromise(async () => {
      this.init(size, connectionString)
      await this._requireNative().connect()
    }, callback)
  }

  open(connectionString, callback) {
    return callbackOrPromise(
      async () => {
        if (!this._native) {
          const config = parseConnectionString(connectionString)
          config.maxConnections = this._maxPoolSize
          this._native = new JsPool(config)
        }

        const validationClient = await this._native.acquire()
        await this._native.release(validationClient)

        const db = new Database(null, async (releasedClient) => {
          if (releasedClient) await this._native.release(releasedClient)
          this._connections.delete(db)
        }, this._native)
        this._connections.add(db)
        return db
      },
      callback
    )
  }

  openSync() {
    return unsupportedSync('Pool.openSync')
  }

  close(callback) {
    if (this._nativeMode) {
      return callbackOrPromise(() => this._requireNative().close(), callback)
    }
    return callbackOrPromise(async () => {
      for (const connection of Array.from(this._connections)) {
        await connection.close().catch(() => {})
      }
      this._connections.clear()
      if (this._native) await this._native.close()
    }, callback)
  }

  closeSync() {
    this.close().catch(() => {})
    return true
  }
}

function debug(value) {
  debugEnabled = Boolean(value)
  return debugEnabled
}

function convertRowsToColumns(rows) {
  if (!Array.isArray(rows) || rows.length === 0) {
    return { params: [], ArraySize: 0 }
  }
  const width = Math.max(...rows.map((row) => (Array.isArray(row) ? row.length : 0)))
  const params = Array.from({ length: width }, (_, columnIndex) => ({
    ParamType: 'ARRAY',
    Data: rows.map((row) => (Array.isArray(row) ? row[columnIndex] : undefined)),
  }))
  return { params, ArraySize: rows.length }
}

const api = {
  ...native,
  JsClient,
  JsPool,
  JsPreparedStatement,
  JsTransaction,
  NativeClient: JsClient,
  NativePool: JsPool,
  NativePreparedStatement: JsPreparedStatement,
  NativeTransaction: JsTransaction,
  Client: JsClient,
  Pool,
  Database,
  ODBCResult,
  ODBCStatement,
  PreparedStatement: JsPreparedStatement,
  Transaction: JsTransaction,
  open,
  openSync,
  debug,
  convertRowsToColumns,
  _compat: {
    parseConnectionString,
    ODBCResult,
    ODBCStatement,
    Database,
  },
}

module.exports = api
module.exports.default = api
