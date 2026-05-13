const { Readable } = require('node:stream')
const native = require('./index.js')

const {
  JsClient,
  JsPool,
  JsPreparedStatement,
  JsTransaction,
} = native

let debugEnabled = false

const SQL_ERROR_RE = /SQLSTATE=([A-Z0-9]+),\s*SQLCODE=(-?\d+)/
const RETRYABLE_SESSION_SQLCODES = new Set([-502, -514, -518])

function enrichDb2Error(error) {
  if (!error || typeof error !== 'object') return error

  const message = String(error.message || error)
  const match = SQL_ERROR_RE.exec(message)
  if (match) {
    if (error.sqlstate == null) error.sqlstate = match[1]
    if (error.sqlcode == null) error.sqlcode = Number(match[2])
  }

  const normalized = message.toLowerCase()
  const retryable =
    RETRYABLE_SESSION_SQLCODES.has(Number(error.sqlcode)) ||
    normalized.includes('closed by server') ||
    normalized.includes('qrynoprm')

  if (error.retryable == null) error.retryable = retryable
  return error
}

function withDb2ErrorEnrichment(promise) {
  return Promise.resolve(promise).catch((error) => {
    throw enrichDb2Error(error)
  })
}

function callbackOrPromise(work, callback, mapValue) {
  const promise = Promise.resolve()
    .then(work)
    .then((value) => (mapValue ? mapValue(value) : value))
    .catch((error) => {
      throw enrichDb2Error(error)
    })

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
    .catch((error) => {
      throw enrichDb2Error(error)
    })

  if (typeof callback === 'function') {
    promise.then(
      (values) => callback(null, ...values),
      (error) => callback(error)
    )
    return undefined
  }

  return promise.then((values) => values[0])
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

function hasConnectionConfig(value) {
  if (!value || typeof value !== 'object') return false
  return Object.keys(value).some((key) => {
    const normalized = normalizeKey(key)
    return (
      normalized === 'HOST' ||
      normalized === 'HOSTNAME' ||
      normalized === 'SERVER' ||
      normalized === 'DATABASE' ||
      normalized === 'DB' ||
      normalized === 'DSN' ||
      normalized === 'UID' ||
      normalized === 'USERID' ||
      normalized === 'USER' ||
      normalized === 'PWD' ||
      normalized === 'PASSWORD'
    )
  })
}

function assignConnectionConfigValue(config, key, value, fromConnectionString = false) {
  if (value == null) return
  const normalized = normalizeKey(key)
  const asNumber = () => Number(value)
  const asString = () => String(value)
  const timeoutMultiplier =
    (fromConnectionString ||
      (key !== 'connectTimeout' && key !== 'queryTimeout' && key !== 'frameDrainTimeout'))
      ? 1000
      : 1

  switch (normalized) {
    case 'HOST':
    case 'HOSTNAME':
    case 'SERVER':
      config.host = asString()
      break
    case 'DATABASE':
    case 'DB':
    case 'DSN':
      config.database = asString()
      break
    case 'UID':
    case 'USERID':
    case 'USER':
      config.user = asString()
      break
    case 'PWD':
    case 'PASSWORD':
      config.password = asString()
      break
    case 'PORT':
      config.port = asNumber()
      break
    case 'SECURITY':
      if (String(value).trim().toLowerCase() === 'ssl') config.ssl = true
      break
    case 'SSL':
      config.ssl = String(value).trim().toLowerCase() === 'ssl' || parseBool(value)
      break
    case 'REJECTUNAUTHORIZED':
      config.rejectUnauthorized = parseBool(value)
      break
    case 'SSLSERVERCERTIFICATE':
    case 'CACERT':
      config.ssl = true
      config.caCert = asString()
      if (normalized === 'SSLSERVERCERTIFICATE' && config.sslClientHostnameValidation == null) {
        config.sslClientHostnameValidation = 'OFF'
      }
      break
    case 'SSLCLIENTHOSTNAMEVALIDATION':
      config.sslClientHostnameValidation = asString()
      break
    case 'SECURITYMECHANISM':
      config.securityMechanism = asString()
      break
    case 'ENCRYPTIONALGORITHM':
      config.encryptionAlgorithm = asString()
      break
    case 'CREDENTIALENCODING':
      config.credentialEncoding = asString()
      break
    case 'ENCRYPTEDPASSWORDENCODING':
      config.encryptedPasswordEncoding = asString()
      break
    case 'ENCRYPTEDPASSWORDTOKENENCODING':
      config.encryptedPasswordTokenEncoding = asString()
      break
    case 'CONNECTTIMEOUT':
      config.connectTimeout = asNumber() * timeoutMultiplier
      break
    case 'QUERYTIMEOUT':
      config.queryTimeout = asNumber() * timeoutMultiplier
      break
    case 'FRAMEDRAINTIMEOUT':
      config.frameDrainTimeout = asNumber() * timeoutMultiplier
      break
    case 'CURRENTSCHEMA':
      config.currentSchema = asString()
      break
    case 'TYPEDEFINITIONNAME':
      config.typeDefinitionName = asString()
      break
    case 'FETCHSIZE':
      config.fetchSize = asNumber()
      break
    case 'MINCONNECTIONS':
      config.minConnections = asNumber()
      break
    case 'MAXCONNECTIONS':
    case 'MAXPOOLSIZE':
      config.maxConnections = asNumber()
      break
    case 'IDLETIMEOUT':
      config.idleTimeout = asNumber()
      break
    case 'MAXLIFETIME':
      config.maxLifetime = asNumber()
      break
    case 'HEALTHCHECKINTERVAL':
      config.healthCheckInterval = asNumber()
      break
  }
}

function applyOpenOptions(config, options = {}) {
  if (!options || typeof options !== 'object') return config
  if (options.connectTimeout != null) {
    config.connectTimeout = Number(options.connectTimeout) * 1000
  }
  if (options.queryTimeout != null) {
    config.queryTimeout = Number(options.queryTimeout) * 1000
  }
  if (options.currentSchema != null) config.currentSchema = options.currentSchema
  if (options.fetchSize != null) config.fetchSize = Number(options.fetchSize)
  if (options.minConnections != null) config.minConnections = Number(options.minConnections)
  if (options.maxConnections != null) config.maxConnections = Number(options.maxConnections)
  if (options.maxPoolSize != null) config.maxConnections = Number(options.maxPoolSize)
  if (options.idleTimeout != null) config.idleTimeout = Number(options.idleTimeout)
  if (options.maxLifetime != null) config.maxLifetime = Number(options.maxLifetime)
  if (options.healthCheckInterval != null) {
    config.healthCheckInterval = Number(options.healthCheckInterval)
  }
  if (options.ssl != null) config.ssl = parseBool(options.ssl)
  if (options.rejectUnauthorized != null) {
    config.rejectUnauthorized = parseBool(options.rejectUnauthorized)
  }
  if (options.sslClientHostnameValidation != null) {
    config.sslClientHostnameValidation = String(options.sslClientHostnameValidation)
  }
  return config
}

function validateConnectionConfig(config) {
  if (config.host && String(config.host).includes(':') && config.port == null) {
    const [host, port] = String(config.host).split(':')
    config.host = host
    config.port = Number(port)
  }

  if (!config.host) {
    throw new Error(
      'HOSTNAME/HOST is required by db2-node; DSN-only local ODBC strings are not supported'
    )
  }
  if (!config.database) throw new Error('DATABASE or DSN is required')
  if (!config.user) throw new Error('UID or USER is required')

  return config
}

function normalizeParam(value) {
  if (value === undefined) return null
  if (value === null) return null
  if (typeof Buffer !== 'undefined' && Buffer.isBuffer(value)) {
    return Array.from(value)
  }
  if (typeof ArrayBuffer !== 'undefined' && ArrayBuffer.isView(value)) {
    return Array.from(new Uint8Array(value.buffer, value.byteOffset, value.byteLength))
  }
  if (typeof ArrayBuffer !== 'undefined' && value instanceof ArrayBuffer) {
    return Array.from(new Uint8Array(value))
  }
  return value
}

function normalizeParams(params) {
  if (!Array.isArray(params)) return undefined
  return params.map(normalizeParam)
}

function normalizeParamRows(paramRows) {
  if (!Array.isArray(paramRows)) return paramRows
  return paramRows.map((params) => normalizeParams(params) || [])
}

function diagnosticsEnabled() {
  return parseBool(process.env.DB2_QUERY_DIAGNOSTICS) || parseBool(process.env.DB2_QUERY_DIAGNOSTICS_STDERR)
}

function emitCompatConnectionDiagnostics(config, source) {
  if (!diagnosticsEnabled()) return
  const host = config.host ? String(config.host) : ''
  const port = config.port == null ? '' : String(config.port)
  const database = config.database ? String(config.database) : ''
  const sslClientHostnameValidation =
    config.sslClientHostnameValidation == null ? 'Basic' : String(config.sslClientHostnameValidation)
  console.error(
    `[db2-diagnostics] compat_connection_config source=${source} host=${host} port=${port} database=${database} ssl=${Boolean(config.ssl)} reject_unauthorized=${config.rejectUnauthorized == null ? 'default' : Boolean(config.rejectUnauthorized)} ca_cert=${Boolean(config.caCert)} ssl_client_hostname_validation=${sslClientHostnameValidation}`
  )
}

function parseConnectionString(connectionString, options = {}) {
  if (connectionString && typeof connectionString === 'object') {
    const config = {}
    for (const [key, value] of Object.entries(connectionString)) {
      assignConnectionConfigValue(config, key, value)
    }
    applyOpenOptions(config, options)
    return validateConnectionConfig(config)
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

  const config = {}
  for (const [key, value] of Object.entries(parts)) {
    assignConnectionConfigValue(config, key, value, true)
  }
  applyOpenOptions(config, options)
  return validateConnectionConfig(config)
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

  return { sql, params: normalizeParams(params), callback: cb, noResults }
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
    return callbackOrPromise(() => {
      if (this._closed) return false
      if (this._offset >= this.rows.length) return false
      return this.rows[this._offset++]
    }, callback)
  }

  fetchAll(option, callback) {
    if (typeof option === 'function') callback = option
    return callbackOrPromise(() => {
      if (this._closed) return []
      const rows = this.rows.slice(this._offset)
      this._offset = this.rows.length
      return rows
    }, callback)
  }

  fetchN(count, option, callback) {
    if (typeof option === 'function') callback = option
    return callbackOrPromise(() => {
      if (this._closed) return []
      const end = Math.min(this._offset + Number(count || 0), this.rows.length)
      const rows = this.rows.slice(this._offset, end)
      this._offset = end
      return rows
    }, callback)
  }

  close(callback) {
    return callbackOrPromise(() => {
      this._closed = true
    }, callback)
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
      this._boundParams = Array.isArray(bindingParameters) ? bindingParameters : []
    }, callback)
  }

  execute(bindingParameters, callback) {
    if (typeof bindingParameters === 'function') {
      callback = bindingParameters
      bindingParameters = undefined
    }
    const params = Array.isArray(bindingParameters) ? bindingParameters : this._boundParams
    return callbackOrPromiseMany(
      async () => new ODBCResult(await this._stmt.execute(normalizeParams(params) || null)),
      callback,
      (result) => [result, undefined]
    )
  }

  executeNonQuery(bindingParameters, callback) {
    if (typeof bindingParameters === 'function') {
      callback = bindingParameters
      bindingParameters = undefined
    }
    const params = Array.isArray(bindingParameters) ? bindingParameters : this._boundParams
    return callbackOrPromise(
      async () => {
        const result = await this._stmt.execute(normalizeParams(params) || null)
        return result.rowCount || 0
      },
      callback
    )
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

}

class Database {
  constructor(client, onClose, pool) {
    this._client = client
    this._onClose = onClose
    this._pool = pool
    this._closed = false
    this._transaction = null
    this.connected = Boolean(client || pool)
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

  open(connectionString, options, callback) {
    if (typeof options === 'function') {
      callback = options
      options = undefined
    }
    return callbackOrPromise(async () => {
      if (this.connected) await this.close()
      const db = await open(connectionString, options)
      this._client = db._client
      this._onClose = db._onClose
      this._pool = db._pool
      this._closed = false
      this._transaction = null
      this.connected = true
      return true
    }, callback)
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

  beginTransaction(callback) {
    return callbackOrPromise(async () => {
      const client = await this._ensureClient()
      this._transaction = await client.beginTransaction()
    }, callback)
  }

  commitTransaction(callback) {
    return callbackOrPromise(async () => {
      if (this._transaction) {
        await this._transaction.commit()
        this._transaction = null
      }
    }, callback)
  }

  rollbackTransaction(callback) {
    return callbackOrPromise(async () => {
      if (this._transaction) {
        await this._transaction.rollback()
        this._transaction = null
      }
    }, callback)
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
      this._pool = null
      this._onClose = null
      this.connected = false
    }, callback)
  }

  setIsolationLevel() {
    return true
  }

  setAttr(attributeName, value, callback) {
    if (typeof value === 'function') callback = value
    return callbackOrPromise(() => true, callback)
  }

  getInfo(infoType, infoLength, callback) {
    if (typeof infoLength === 'function') callback = infoLength
    return callbackOrPromise(async () => {
      const client = await this._ensureClient()
      return client.serverInfo()
    }, callback)
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
      emitCompatConnectionDiagnostics(config, 'open')
      const maxConnections = config.maxConnections == null ? 2 : config.maxConnections
      const minConnections = config.minConnections == null ? Math.min(2, maxConnections) : config.minConnections
      const pool = new JsPool({ ...config, minConnections, maxConnections })
      await pool.connect()
      return new Database(null, async (releasedClient) => {
        if (releasedClient) await pool.release(releasedClient)
        await pool.close()
      }, pool)
    },
    callback
  )
}

function stableConfigKey(config) {
  const sorted = {}
  for (const key of Object.keys(config).sort()) {
    if (config[key] !== undefined) sorted[key] = config[key]
  }
  return JSON.stringify(sorted)
}

function connectionIdentityKey(config) {
  const {
    minConnections,
    maxConnections,
    idleTimeout,
    maxLifetime,
    healthCheckInterval,
    ...identity
  } = config
  return stableConfigKey(identity)
}

class CompatPool {
  constructor(config) {
    this._maxPoolSize = 10
    this._connections = new Set()
    this._native = null
    this._nativeMode = false
    this._pools = new Map()
    this._connectionKey = null
    this._openOptions = {}

    if (config) {
      if (hasConnectionConfig(config)) {
        const nativeConfig = parseConnectionString(config)
        this._maxPoolSize = nativeConfig.maxConnections || this._maxPoolSize
        this._native = new JsPool(nativeConfig)
        this._nativeMode = true
        this._connectionKey = connectionIdentityKey(nativeConfig)
        this._pools.set(this._connectionKey, {
          pool: this._native,
          config: nativeConfig,
        })
      } else {
        this._applyPoolOptions(config)
      }
    }
  }

  _applyPoolOptions(options) {
    if (!options || typeof options !== 'object') return
    if (options.maxPoolSize != null) this._maxPoolSize = Number(options.maxPoolSize) || this._maxPoolSize
    if (options.maxConnections != null) this._maxPoolSize = Number(options.maxConnections) || this._maxPoolSize
    if (options.connectTimeout != null) this._openOptions.connectTimeout = Number(options.connectTimeout)
    if (options.queryTimeout != null) this._openOptions.queryTimeout = Number(options.queryTimeout)
    if (options.currentSchema != null) this._openOptions.currentSchema = options.currentSchema
    if (options.fetchSize != null) this._openOptions.fetchSize = Number(options.fetchSize)
    if (options.idleTimeout != null) this._openOptions.idleTimeout = Number(options.idleTimeout)
    if (options.maxLifetime != null) this._openOptions.maxLifetime = Number(options.maxLifetime)
    if (options.healthCheckInterval != null) {
      this._openOptions.healthCheckInterval = Number(options.healthCheckInterval)
    }
    if (options.ssl != null) this._openOptions.ssl = options.ssl
    if (options.rejectUnauthorized != null) {
      this._openOptions.rejectUnauthorized = options.rejectUnauthorized
    }
    if (options.sslClientHostnameValidation != null) {
      this._openOptions.sslClientHostnameValidation = options.sslClientHostnameValidation
    }
  }

  _requireNative() {
    if (!this._native) {
      throw new Error('Pool is not initialized; pass a connection config to new Pool(config) or call init/initAsync/open first')
    }
    return this._native
  }

  _getOrCreateNativePool(connectionString, source, overrides = {}) {
    const config = parseConnectionString(connectionString, this._openOptions)
    Object.assign(config, overrides)
    if (config.maxConnections == null) config.maxConnections = this._maxPoolSize
    const key = connectionIdentityKey(config)
    let entry = this._pools.get(key)
    if (!entry) {
      emitCompatConnectionDiagnostics(config, source)
      if (this._native && !this._connectionKey) {
        entry = {
          pool: this._native,
          config,
        }
        this._pools.set(key, entry)
        this._connectionKey = key
        return entry
      }
      entry = {
        pool: new JsPool(config),
        config,
      }
      this._pools.set(key, entry)
    }
    this._native = entry.pool
    this._connectionKey = key
    return entry
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
    return callbackOrPromise(() => this._requireNative().query(sql, normalizeParams(params) || null), callback)
  }

  acquire(callback) {
    return callbackOrPromise(
      async () => Client.fromNative(await this._requireNative().acquire()),
      callback
    )
  }

  release(client, callback) {
    return callbackOrPromise(
      () => this._requireNative().release(client && client._native ? client._native : client),
      callback
    )
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
      return callbackOrPromise(() => this._native ? this._native.maxConnections() : this._maxPoolSize, callback)
    }
    return this._native ? this._native.maxConnections() : this._maxPoolSize
  }

  setMaxPoolSize(size) {
    this._maxPoolSize = Number(size) || this._maxPoolSize
    return true
  }

  setConnectTimeout(timeout) {
    this._openOptions.connectTimeout = Number(timeout)
    return true
  }

  init(size, connectionString) {
    this._maxPoolSize = Number(size) || this._maxPoolSize
    this._getOrCreateNativePool(connectionString, 'pool_init', {
      minConnections: this._maxPoolSize,
      maxConnections: this._maxPoolSize,
    })
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
        const { pool } = this._getOrCreateNativePool(connectionString, 'pool_open')

        const validationClient = await pool.acquire()
        await pool.release(validationClient)

        const db = new Database(null, async (releasedClient) => {
          if (releasedClient) await pool.release(releasedClient)
          this._connections.delete(db)
        }, pool)
        this._connections.add(db)
        return db
      },
      callback
    )
  }

  close(callback) {
    return callbackOrPromise(async () => {
      for (const connection of Array.from(this._connections)) {
        await connection.close().catch(() => {})
      }
      this._connections.clear()
      const pools = new Set(Array.from(this._pools.values()).map((entry) => entry.pool))
      if (this._native) pools.add(this._native)
      for (const pool of pools) {
        await pool.close()
      }
      this._pools.clear()
      this._native = null
      this._connectionKey = null
      this._nativeMode = false
    }, callback)
  }

}

class PreparedStatement {
  constructor(stmt) {
    this._native = stmt
  }

  static fromNative(stmt) {
    return new PreparedStatement(stmt)
  }

  execute(params) {
    return withDb2ErrorEnrichment(this._native.execute(normalizeParams(params) || null))
  }

  executeBatch(paramRows) {
    return withDb2ErrorEnrichment(this._native.executeBatch(normalizeParamRows(paramRows)))
  }

  close() {
    return withDb2ErrorEnrichment(this._native.close())
  }
}

class Transaction {
  constructor(transaction) {
    this._native = transaction
  }

  static fromNative(transaction) {
    return new Transaction(transaction)
  }

  query(sql, params) {
    return withDb2ErrorEnrichment(this._native.query(sql, normalizeParams(params) || null))
  }

  async prepare(sql) {
    return PreparedStatement.fromNative(await withDb2ErrorEnrichment(this._native.prepare(sql)))
  }

  commit() {
    return withDb2ErrorEnrichment(this._native.commit())
  }

  rollback() {
    return withDb2ErrorEnrichment(this._native.rollback())
  }
}

class Client {
  constructor(config) {
    this._native = new JsClient(config)
  }

  static fromNative(client) {
    const wrapper = Object.create(Client.prototype)
    wrapper._native = client
    return wrapper
  }

  connect() {
    return withDb2ErrorEnrichment(this._native.connect())
  }

  query(sql, params) {
    return withDb2ErrorEnrichment(this._native.query(sql, normalizeParams(params) || null))
  }

  async prepare(sql) {
    return PreparedStatement.fromNative(await withDb2ErrorEnrichment(this._native.prepare(sql)))
  }

  async beginTransaction() {
    return Transaction.fromNative(await withDb2ErrorEnrichment(this._native.beginTransaction()))
  }

  close() {
    return withDb2ErrorEnrichment(this._native.close())
  }

  serverInfo() {
    return this._native.serverInfo()
  }
}

class Pool {
  constructor(config) {
    this._native = new JsPool(config)
  }

  connect() {
    return withDb2ErrorEnrichment(this._native.connect())
  }

  warmup() {
    return withDb2ErrorEnrichment(this._native.warmup())
  }

  query(sql, params) {
    return withDb2ErrorEnrichment(this._native.query(sql, normalizeParams(params) || null))
  }

  async acquire() {
    return Client.fromNative(await withDb2ErrorEnrichment(this._native.acquire()))
  }

  release(client) {
    return withDb2ErrorEnrichment(
      this._native.release(client && client._native ? client._native : client)
    )
  }

  close() {
    return withDb2ErrorEnrichment(this._native.close())
  }

  idleCount() {
    return this._native.idleCount()
  }

  activeCount() {
    return this._native.activeCount()
  }

  totalCount() {
    return this._native.totalCount()
  }

  maxConnections() {
    return this._native.maxConnections()
  }
}

function createDatabase() {
  return new Database()
}

function debug(value) {
  debugEnabled = Boolean(value)
  return debugEnabled
}

function closeDatabaseHandle(db) {
  if (db && typeof db === 'object') {
    for (const key of Object.keys(db)) {
      delete db[key]
    }
  }
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

const api = Object.assign(createDatabase, {
  ...native,
  JsClient,
  JsPool,
  JsPreparedStatement,
  JsTransaction,
  NativeClient: JsClient,
  NativePool: JsPool,
  NativePreparedStatement: JsPreparedStatement,
  NativeTransaction: JsTransaction,
  Client,
  Pool: CompatPool,
  Db2Pool: Pool,
  CompatPool,
  IbmDbPool: CompatPool,
  Database,
  ODBCResult,
  ODBCStatement,
  PreparedStatement,
  Transaction,
  open,
  close: closeDatabaseHandle,
  debug,
  convertRowsToColumns,
  _compat: {
    parseConnectionString,
    enrichDb2Error,
    ODBCResult,
    ODBCStatement,
    Database,
    Pool: CompatPool,
  },
})

module.exports = api
module.exports.default = api
