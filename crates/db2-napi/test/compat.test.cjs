const test = require('node:test')
const assert = require('node:assert/strict')

const ibmdb = require('..')

test('exports ibm_db-compatible entry points', () => {
  assert.equal(typeof ibmdb.open, 'function')
  assert.equal(typeof ibmdb.openSync, 'function')
  assert.equal(typeof ibmdb.Pool, 'function')
  assert.equal(typeof ibmdb.Database, 'function')
  assert.equal(typeof ibmdb.ODBCResult, 'function')
  assert.equal(typeof ibmdb.NativePool, 'function')
})

test('parses common ibm_db connection string keywords', () => {
  const config = ibmdb._compat.parseConnectionString(
    'DATABASE=DDFIC0A;HOSTNAME=db.example.com;PORT=3380;PROTOCOL=TCPIP;UID=user;PWD=pass;Security=SSL;CURRENTSCHEMA=APP',
    { connectTimeout: 40 }
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
