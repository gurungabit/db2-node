import assert from 'node:assert/strict'
import { createRequire } from 'node:module'
import { describe, it } from 'node:test'
const require = createRequire(import.meta.url)
const packagePath = '../../crates/db2-napi'
const esmEntrypoint = new URL('../../crates/db2-napi/index.mjs', import.meta.url)

describe('Package exports', () => {
  it('supports CommonJS consumers with friendly aliases', () => {
    const pkg = require(packagePath)

    assert.equal(pkg.Client, pkg.JsClient)
    assert.equal(pkg.Pool, pkg.JsPool)
    assert.equal(pkg.PreparedStatement, pkg.JsPreparedStatement)
    assert.equal(pkg.Transaction, pkg.JsTransaction)
    assert.equal(typeof pkg.Client, 'function')
    assert.equal(typeof pkg.Pool, 'function')
  })

  it('supports ESM consumers with friendly aliases', async () => {
    const pkg = await import(esmEntrypoint.href)

    assert.equal(pkg.Client, pkg.JsClient)
    assert.equal(pkg.Pool, pkg.JsPool)
    assert.equal(pkg.PreparedStatement, pkg.JsPreparedStatement)
    assert.equal(pkg.Transaction, pkg.JsTransaction)
    assert.equal(pkg.default.Client, pkg.Client)
  })
})
