const fs = require('node:fs')
const path = require('node:path')

const MACHO_64_MAGIC = 0xfeedfacf
const CPU_TYPE_ARM64 = 0x0100000c

function verifyDarwinArm64(file) {
  const resolved = path.resolve(file)
  const header = fs.readFileSync(resolved).subarray(0, 12)

  if (header.length < 12) {
    throw new Error(`${file} is too short to be a Mach-O binary`)
  }
  if (header.readUInt32LE(0) !== MACHO_64_MAGIC) {
    throw new Error(`${file} is not a little-endian 64-bit Mach-O binary`)
  }
  if (header.readUInt32LE(4) !== CPU_TYPE_ARM64) {
    throw new Error(`${file} is not an ARM64 Mach-O binary`)
  }

  return resolved
}

if (require.main === module) {
  const file = process.argv[2] || 'db2-node.darwin-arm64.node'
  const resolved = verifyDarwinArm64(file)
  console.log(`Verified macOS ARM64 binding: ${resolved}`)
}

module.exports = { verifyDarwinArm64 }
