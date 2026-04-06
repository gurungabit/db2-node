import { createRequire } from 'node:module'

const require = createRequire(import.meta.url)
const native = require('./index.js')

export const JsClient = native.JsClient
export const JsPool = native.JsPool
export const JsPreparedStatement = native.JsPreparedStatement
export const JsTransaction = native.JsTransaction

export const Client = native.JsClient
export const Pool = native.JsPool
export const PreparedStatement = native.JsPreparedStatement
export const Transaction = native.JsTransaction

export default {
  JsClient,
  JsPool,
  JsPreparedStatement,
  JsTransaction,
  Client,
  Pool,
  PreparedStatement,
  Transaction,
}
