const test = require('node:test')
const assert = require('node:assert/strict')

const ibmdb = require('..')

test('exports ibm_db-compatible entry points', () => {
  assert.equal(typeof ibmdb, 'function')
  assert.equal(typeof ibmdb.open, 'function')
  assert.deepEqual(Object.keys(ibmdb).filter((key) => /ync$/.test(key)), [])
  assert.equal(typeof ibmdb.Pool, 'function')
  assert.notEqual(ibmdb.Pool, ibmdb.NativePool)
  assert.equal(typeof ibmdb.Db2Pool, 'function')
  assert.equal(typeof ibmdb.CompatPool, 'function')
  assert.equal(typeof ibmdb.Database, 'function')
  assert.equal(typeof ibmdb.ODBCResult, 'function')
  assert.equal(typeof ibmdb.NativePool, 'function')
  assert.ok(ibmdb() instanceof ibmdb.Database)
})

test('parses common ibm_db connection string keywords', () => {
  const config = ibmdb._compat.parseConnectionString(
    'DATABASE=DBX9Q2A;HOSTNAME=db.example.com;PORT=3380;PROTOCOL=TCPIP;UID=user;PWD=pass;Security=SSL;CURRENTSCHEMA=APP;SSLServerCertificate=/tmp/db2-ca.pem',
    { connectTimeout: 40, minConnections: 2, maxConnections: 4 }
  )

  assert.deepEqual(config, {
    host: 'db.example.com',
    database: 'DBX9Q2A',
    user: 'user',
    password: 'pass',
    port: 3380,
    ssl: true,
    caCert: '/tmp/db2-ca.pem',
    sslClientHostnameValidation: 'OFF',
    currentSchema: 'APP',
    connectTimeout: 40000,
    minConnections: 2,
    maxConnections: 4,
  })
})

test('explicit SSLClientHostnameValidation overrides SSLServerCertificate default', () => {
  const config = ibmdb._compat.parseConnectionString(
    'DATABASE=DBX9Q2A;HOSTNAME=db.example.com;UID=user;PWD=pass;Security=SSL;SSLServerCertificate=/tmp/db2-ca.pem;SSLClientHostnameValidation=Basic'
  )

  assert.equal(config.ssl, true)
  assert.equal(config.caCert, '/tmp/db2-ca.pem')
  assert.equal(config.sslClientHostnameValidation, 'Basic')
})

test('parses ibm_db-style connection objects', () => {
  const config = ibmdb._compat.parseConnectionString({
    DATABASE: 'DBX9Q2A',
    HOSTNAME: 'db.example.com:3380',
    UID: 'user',
    PWD: 'pass',
    Security: 'SSL',
    CurrentSchema: 'APP',
    ConnectTimeout: 40,
  })

  assert.deepEqual(config, {
    host: 'db.example.com',
    database: 'DBX9Q2A',
    user: 'user',
    password: 'pass',
    ssl: true,
    currentSchema: 'APP',
    connectTimeout: 40000,
    port: 3380,
  })
})

test('parses IBM SSLClientHostnameValidation=OFF connection string keyword', () => {
  const config = ibmdb._compat.parseConnectionString(
    'DATABASE=DBX9Q2A;HOSTNAME=db.example.com;UID=user;PWD=pass;Security=SSL;SSLClientHostnameValidation=OFF'
  )

  assert.equal(config.ssl, true)
  assert.equal(config.sslClientHostnameValidation, 'OFF')
})

test('memory ODBCResult supports async fetch APIs', async () => {
  const result = new ibmdb.ODBCResult({
    rows: [{ A: 1 }, { A: 2 }],
    columns: [{ name: 'A', typeName: 'INTEGER', nullable: false }],
    rowCount: 2,
  })

  assert.deepEqual(await result.fetch(), { A: 1 })
  assert.deepEqual(await result.fetchAll(), [{ A: 2 }])
  assert.equal(await result.fetch(), false)
})

test('db2 errors expose sqlstate sqlcode and retryable metadata', async () => {
  const err = new Error('SQL Error [SQLSTATE=26501, SQLCODE=-514]: SQL_CURSH200C2')

  assert.equal(ibmdb._compat.enrichDb2Error(err), err)
  assert.equal(err.sqlstate, '26501')
  assert.equal(err.sqlcode, -514)
  assert.equal(err.retryable, true)

  const client = Object.create(ibmdb.Client.prototype)
  client._native = {
    query: async () => {
      throw new Error('z/OS LOB base failed: SQL Error [SQLSTATE=26501, SQLCODE=-514]: SQL_CURSH200C2')
    },
  }

  await assert.rejects(client.query('SELECT * FROM T'), (error) => {
    assert.equal(error.sqlstate, '26501')
    assert.equal(error.sqlcode, -514)
    assert.equal(error.retryable, true)
    return true
  })
})

test('db2 error enrichment leaves unrelated errors non-retryable', () => {
  const err = new Error('query ended with undecoded row data')

  ibmdb._compat.enrichDb2Error(err)
  assert.equal(err.sqlstate, undefined)
  assert.equal(err.sqlcode, undefined)
  assert.equal(err.retryable, false)
})

test('ibm_db Pool constructor does not require connection config', async () => {
  const pool = new ibmdb.Pool()
  assert.equal(pool.maxConnections(), 10)
  await assert.rejects(pool.connect(), /Pool is not initialized/)

  const sized = new ibmdb.Pool({ maxPoolSize: 3 })
  assert.equal(sized.maxConnections(), 3)
})

test('compat Pool.connect propagates async errors to callback and promise callers', async () => {
  const pool = new ibmdb.CompatPool()
  const failure = new Error('bad credentials')
  pool._native = {
    connect: async () => {
      throw failure
    },
  }

  await assert.rejects(pool.connect(), /bad credentials/)

  await new Promise((resolve, reject) => {
    pool.connect((error) => {
      try {
        assert.equal(error, failure)
        resolve()
      } catch (assertionError) {
        reject(assertionError)
      }
    })
  })
})

test('compat Pool.open simple query uses native Pool.query fast path', async () => {
  const pool = new ibmdb.CompatPool()
  const calls = []
  pool._native = {
    acquire: async () => {
      calls.push('acquire')
      return {
        query: async () => {
          throw new Error('sticky client query should not be used for simple pool queries')
        },
      }
    },
    release: async () => {
      calls.push('release')
    },
    query: async (sql, params) => {
      calls.push(['query', sql, params])
      return {
        rows: [{ A: 1 }],
        columns: [{ name: 'A', typeName: 'INTEGER', nullable: false }],
        rowCount: 1,
        diagnostics: [],
      }
    },
  }

  const db = await pool.open('DATABASE=D;HOSTNAME=h;UID=u;PWD=p')
  const rows = await db.query('VALUES ?', [1])

  assert.deepEqual(rows, [{ A: 1 }])
  assert.deepEqual(calls, ['acquire', 'release', ['query', 'VALUES ?', [1]]])
  await db.close()
})
