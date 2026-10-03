import test from 'node:test';
import assert from 'node:assert/strict';
import net from 'node:net';
import { Client, getConfig, tempTable } from './helpers';
import db2 from '../../crates/db2-napi/index.cjs';

const config = getConfig();

test('LUW issue #12 follow-up regressions (16–21)', { timeout: 180_000 }, async (t) => {
  const check = async (name: string, run: (c: InstanceType<typeof Client>) => Promise<void>) => {
    await t.test(name, async () => {
      const c = new Client(config);
      await c.connect();
      try { await run(c); } finally { await c.close(); }
    });
  };
  await check('16: leading comments work for direct/prepared SELECT and CALL', async (c) => {
    for (const prefix of ['/* note */ ', '-- note\n', ' -- first\r\n /* second */\n']) {
      const sql = `${prefix}SELECT 1 AS X FROM SYSIBM.SYSDUMMY1`;
      assert.deepEqual((await c.query(sql)).rows, [{ X: 1 }]);
      const statement = await c.prepare(sql);
      try { assert.deepEqual((await statement.execute()).rows, [{ X: 1 }]); }
      finally { await statement.close(); }
    }
    assert.deepEqual((await c.query("/* note */ VALUES CAST(? AS INTEGER)", [7])).rows, [{ '1': 7 }]);
    const table = tempTable('ISSUE12_COMMENT');
    await c.query(`CREATE TABLE ${table}(ID INTEGER)`);
    try { await c.query(`/* note */ CALL SYSPROC.ADMIN_CMD('RUNSTATS ON TABLE DB2INST1.${table}')`); }
    finally { await c.query(`DROP TABLE ${table}`); }
  });
  await check('17: unmatched UPDATE returns zero rows', async (c) => {
    const table = tempTable('ISSUE12_UPDATE');
    await c.query(`CREATE TABLE ${table}(ID INTEGER, SALARY INTEGER)`);
    try {
      await c.query(`INSERT INTO ${table} VALUES(1,10)`);
      assert.equal((await c.query(`UPDATE ${table} SET SALARY=SALARY WHERE ID=-9999`)).rowCount, 0);
      const statement = await c.prepare(`UPDATE ${table} SET SALARY=SALARY WHERE ID=?`);
      try {
        assert.equal((await statement.execute([-9999])).rowCount, 0);
        assert.equal((await statement.execute([1])).rowCount, 1);
      } finally { await statement.close(); }
    } finally { await c.query(`DROP TABLE ${table}`); }
  });
  await check('18: a truncation warning preserves its value and connection', async (c) => {
    const r = await c.query("VALUES CAST(REPEAT('x', 100) AS VARCHAR(10))");
    assert.deepEqual(r.rows, [{ '1': 'xxxxxxxxxx' }]);
    assert.equal(r.rowCount, 1);
    const statement = await c.prepare("VALUES CAST(REPEAT('x', 100) AS VARCHAR(10))");
    try { assert.deepEqual((await statement.execute()).rows, [{ '1': 'xxxxxxxxxx' }]); }
    finally { await statement.close(); }
    assert.deepEqual((await c.query("VALUES (1, CAST(REPEAT('x',100) AS VARCHAR(10)))")).rows, [{ '1': 1, '2': 'xxxxxxxxxx' }]);
    assert.equal((await c.query('VALUES 1')).rows[0]['1'], 1);
  });
  await check('19: rows spanning several DRDA blocks stay complete', async (c) => {
    const sql = "SELECT CAST(REPEAT('a',32672) AS VARCHAR(32672)) AS A, CAST(REPEAT('b',32672) AS VARCHAR(32672)) AS B FROM SYSIBM.SYSDUMMY1";
    const verify = (r: any) => {
      assert.equal(r.rowCount, 1);
      assert.equal(r.rows[0].A, 'a'.repeat(32672));
      assert.equal(r.rows[0].B, 'b'.repeat(32672));
    };
    verify(await c.query(sql));
    const statement = await c.prepare(sql);
    try { verify(await statement.execute()); } finally { await statement.close(); }
    const table = tempTable('ISSUE12_WIDE');
    await c.query(`CREATE TABLE ${table}(ID INTEGER, A VARCHAR(32672), B VARCHAR(32672))`);
    try {
      await c.query(`INSERT INTO ${table} VALUES(1,REPEAT('a',32672),REPEAT('b',32672)),(2,REPEAT('a',32672),REPEAT('b',32672)),(3,NULL,NULL)`);
      const checkRows = (r: any) => {
        assert.equal(r.rowCount, 3);
        assert.deepEqual(r.rows.map((row: any) => row.ID), [1,2,3]);
        for (const row of r.rows.slice(0,2)) {
          assert.equal(row.A, 'a'.repeat(32672));
          assert.equal(row.B, 'b'.repeat(32672));
        }
        assert.equal(r.rows[2].A, null);
        assert.equal(r.rows[2].B, null);
      };
      checkRows(await c.query(`SELECT * FROM ${table} ORDER BY ID`));
      const wide = await c.prepare(`SELECT * FROM ${table} ORDER BY ID`);
      try { checkRows(await wide.execute()); } finally { await wide.close(); }
    } finally { await c.query(`DROP TABLE ${table}`); }
    // Coalesce metadata and wide-row replies as Linux TCP reads can do.
    // Forward server responses only; no payloads or credentials are recorded.
    const sockets = new Set<net.Socket>();
    const proxy = net.createServer((socket) => {
      const upstream = net.connect(config.port!, config.host);
      sockets.add(socket); sockets.add(upstream);
      socket.pipe(upstream);
      let pending: Buffer[] = [];
      let timer: ReturnType<typeof setTimeout> | undefined;
      upstream.on('data', (data) => {
        pending.push(data);
        clearTimeout(timer);
        timer = setTimeout(() => {
          socket.write(Buffer.concat(pending));
          pending = [];
        }, 10);
      });
      socket.on('close', () => { clearTimeout(timer); upstream.destroy(); });
      socket.on('error', () => upstream.destroy());
      upstream.on('error', () => socket.destroy());
    });
    await new Promise<void>((resolve, reject) => {
      proxy.once('error', reject);
      proxy.listen(0, '127.0.0.1', resolve);
    });
    const coalesced = new Client({ ...config, host: '127.0.0.1', port: (proxy.address() as net.AddressInfo).port });
    try {
      await coalesced.connect();
      verify(await coalesced.query(sql));
      const wide = await coalesced.prepare(sql);
      try { verify(await wide.execute()); } finally { await wide.close(); }
      assert.equal((await coalesced.query('VALUES 1')).rows[0]['1'], 1);
    } finally {
      await coalesced.close().catch(() => {});
      for (const socket of sockets) socket.destroy();
      await new Promise<void>((resolve) => proxy.close(() => resolve()));
    }
    assert.equal((await c.query('VALUES 1')).rows[0]['1'], 1);
  });
  await check('20: invalid/overflowing DECIMAL parameters fail without changing stored data', async (c) => {
    const table = tempTable('ISSUE12_DEC');
    await c.query(`CREATE TABLE ${table}(ID INTEGER, AMT DECIMAL(5,2))`);
    await c.query(`INSERT INTO ${table} VALUES(1,12.34)`);
    try {
      for (const value of ['12345.67', '99999', 'abc', '1.2.3']) {
        await assert.rejects(c.query(`UPDATE ${table} SET AMT=? WHERE ID=?`, [value, 1]), /decimal|range|precision|numeric/i);
        assert.equal((await c.query(`SELECT AMT FROM ${table}`)).rows[0].AMT, '12.34');
      }
      for (const value of [99999, 'abc', '1e2', '', '.', '--1']) {
        await assert.rejects(c.query('VALUES CAST(? AS DECIMAL(5,2))', [value]), /decimal|range|precision|numeric/i);
      }
      assert.equal((await c.query('VALUES CAST(? AS DECIMAL(5,2))', ['999.99'])).rows[0]['1'], '999.99');
    } finally { await c.query(`DROP TABLE ${table}`); }
  });
  await check('21: nested bigint/objects are catchable errors; Date is UTC text', async (c) => {
    const native = new db2.NativeClient(config);
    await native.connect();
    try {
      for (const client of [c, native]) {
        for (const value of [[1n], { a: 1n }, new Map([['a', 1]]), [256], [-1], [1.5], [[1]]]) {
          await assert.rejects(async () => client.query('VALUES CAST(? AS VARCHAR(100))', [value]), /parameter|byte|object/i);
          await assert.rejects(async () => client.query('VALUES CAST(? AS BLOB(100))', [value]), /parameter|byte|object/i);
        }
        assert.equal((await client.query('VALUES CAST(? AS VARCHAR(100))', [new Date('2024-02-29T23:59:59.123Z')])).rows[0]['1'], '2024-02-29 23:59:59.123');
        assert.equal((await client.query('VALUES LENGTH(CAST(? AS BLOB(100)))', [[0,127,255]])).rows[0]['1'], 3);
        assert.equal((await client.query('VALUES 1')).rows[0]['1'], 1);
      }
    } finally { await native.close(); }
  });
});
