import { createRequire } from 'node:module'

const require = createRequire(import.meta.url)
const compat = require('./index.cjs')

export const JsClient = compat.JsClient
export const JsPool = compat.JsPool
export const JsPreparedStatement = compat.JsPreparedStatement
export const JsTransaction = compat.JsTransaction

export const NativeClient = compat.NativeClient
export const NativePool = compat.NativePool
export const NativePreparedStatement = compat.NativePreparedStatement
export const NativeTransaction = compat.NativeTransaction

export const Client = compat.Client
export const Pool = compat.Pool
export const Database = compat.Database
export const ODBCResult = compat.ODBCResult
export const ODBCStatement = compat.ODBCStatement
export const PreparedStatement = compat.PreparedStatement
export const Transaction = compat.Transaction

export const open = compat.open
export const openSync = compat.openSync
export const debug = compat.debug
export const convertRowsToColumns = compat.convertRowsToColumns

export default compat
