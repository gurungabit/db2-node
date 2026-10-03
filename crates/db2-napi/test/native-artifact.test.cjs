const assert = require('node:assert/strict')
const fs = require('node:fs')
const os = require('node:os')
const path = require('node:path')
const test = require('node:test')
const { spawnSync } = require('node:child_process')
const { verifyDarwinArm64 } = require('../scripts/verify-darwin-arm64.cjs')

function temporaryFile(t, header) {
  const directory = fs.mkdtempSync(path.join(os.tmpdir(), 'db2-node-native-'))
  const file = path.join(directory, 'binding.node')
  fs.writeFileSync(file, header)
  t.after(() => fs.rmSync(directory, { recursive: true, force: true }))
  return file
}

test('accepts a 64-bit ARM64 Mach-O header', (t) => {
  const header = Buffer.alloc(12)
  header.writeUInt32LE(0xfeedfacf, 0)
  header.writeUInt32LE(0x0100000c, 4)

  const file = temporaryFile(t, header)

  assert.equal(verifyDarwinArm64(file), file)
})

test('rejects a non-ARM64 Mach-O header', (t) => {
  const header = Buffer.alloc(12)
  header.writeUInt32LE(0xfeedfacf, 0)
  header.writeUInt32LE(0x01000007, 4)

  const file = temporaryFile(t, header)

  assert.throws(() => verifyDarwinArm64(file), /not an ARM64 Mach-O binary/)
})

test('rejects a file without a Mach-O header', (t) => {
  const file = temporaryFile(t, Buffer.alloc(12))

  assert.throws(() => verifyDarwinArm64(file), /not a little-endian 64-bit Mach-O binary/)
})


test('prepublish rejects an empty comma-separated required binary list', () => {
  const result = spawnSync(process.execPath, [path.resolve(__dirname, '../scripts/verify-prepublish.cjs')], {
    env: { ...process.env, DB2_NODE_REQUIRED_BINARIES: ' , , ' },
    encoding: 'utf8',
  })
  assert.equal(result.status, 1)
  assert.match(result.stderr, /must name at least one binary/)
})
