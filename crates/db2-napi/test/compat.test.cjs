const test = require('node:test')
const assert = require('node:assert/strict')

const ibmdb = require('..')

test('exports ibm_db-compatible entry points', () => {
  assert.equal(typeof ibmdb.open, 'function')
  assert.equal(typeof ibmdb.openSync, 'function')
  assert.equal(typeof ibmdb.Pool, 'function')
  assert.equal(ibmdb.Pool, ibmdb.NativePool)
  assert.equal(typeof ibmdb.CompatPool, 'function')
  assert.equal(typeof ibmdb.Database, 'function')
  assert.equal(typeof ibmdb.ODBCResult, 'function')
  assert.equal(typeof ibmdb.NativePool, 'function')
})

test('parses common ibm_db connection string keywords', () => {
  const config = ibmdb._compat.parseConnectionString(
    'DATABASE=DDFIC0A;HOSTNAME=db.example.com;PORT=3380;PROTOCOL=TCPIP;UID=user;PWD=pass;Security=SSL;CURRENTSCHEMA=APP',
    { connectTimeout: 40, minConnections: 2, maxConnections: 4 }
  )

  assert.deepEqual(config, {
    host: 'db.example.com',
    database: 'DDFIC0A',
    user: 'user',
    password: 'pass',
    port: 3380,
    ssl: true,
    currentSchema: 'APP',
    connectTimeout: 40000,
    minConnections: 2,
    maxConnections: 4,
  })
})

test('memory ODBCResult supports fetch APIs', () => {
  const result = new ibmdb.ODBCResult({
    rows: [{ A: 1 }, { A: 2 }],
    columns: [{ name: 'A', typeName: 'INTEGER', nullable: false }],
    rowCount: 2,
  })

  assert.deepEqual(result.fetchSync(), { A: 1 })
  assert.deepEqual(result.fetchAllSync(), [{ A: 2 }])
  assert.equal(result.fetchSync(), false)
  assert.deepEqual(result.getColumnNamesSync(), ['A'])
})

test('sync database APIs fail loudly instead of pretending to block', () => {
  assert.throws(() => ibmdb.openSync('DATABASE=D;HOSTNAME=h;UID=u;PWD=p'), /not supported/)
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

  const db = await pool.open('unused')
  const rows = await db.query('VALUES ?', [1])

  assert.deepEqual(rows, [{ A: 1 }])
  assert.deepEqual(calls, ['acquire', 'release', ['query', 'VALUES ?', [1]]])
  await db.close()
})
