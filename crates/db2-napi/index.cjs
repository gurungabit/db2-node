const native = require('./index.js')

module.exports = native
module.exports.Client = native.JsClient
module.exports.Pool = native.JsPool
module.exports.PreparedStatement = native.JsPreparedStatement
module.exports.Transaction = native.JsTransaction
module.exports.default = module.exports
