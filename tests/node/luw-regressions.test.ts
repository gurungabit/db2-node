import test from 'node:test';
import assert from 'node:assert/strict';
import { Client, getConfig } from './helpers';
import db2 from '../../crates/db2-napi/index.cjs';
import { setTimeout as delay } from 'node:timers/promises';

// GitHub issues #11 and #12: run against the stock UTF-8 Docker database.
const config = { ...getConfig(), securityMechanism: 'userPassword' };

test('LUW issue regressions', { timeout: 300_000 }, async (t) => {
  const table = `ISSUE12_${Date.now() % 1_000_000}`;
  const setup = new Client(config);
  await setup.connect();
  await setup.query(`CREATE TABLE ${table} (ID INTEGER NOT NULL PRIMARY KEY,
    B BOOLEAN, TS0 TIMESTAMP(0), TS12 TIMESTAMP(12), CL CLOB(1K), BIG BIGINT, X XML)`);
  await setup.query(`INSERT INTO ${table} VALUES (1, TRUE, '2024-02-29-23.59.59',
    '2024-02-29-23.59.59.123456789012', 'clob text', 9223372036854775807, '<a>1</a>')`);
  await setup.query(`INSERT INTO ${table} (ID, B) VALUES (2, FALSE)`);
  await setup.query(`INSERT INTO ${table} (ID) VALUES (3)`);
  await setup.close();

  const check = async (name: string, run: (client: InstanceType<typeof Client>) => Promise<void>) => {
    await t.test(name, async () => {
      const client = new Client(config);
      await client.connect();
      try { await run(client); } finally { await client.close(); }
    });
  };

  try {
    await check('UTF-8 literals and parameters round trip', async (c) => {
      assert.equal((await c.query("SELECT CAST('Grüße 世界' AS VARCHAR(40)) AS V FROM SYSIBM.SYSDUMMY1")).rows[0].V, 'Grüße 世界');
      assert.equal((await c.query('SELECT CAST(? AS VARCHAR(40)) AS V FROM SYSIBM.SYSDUMMY1', ['héllo'])).rows[0].V, 'héllo');
    });
    await check('mixed numeric types keep little-endian integers', async (c) => {
      const r = await c.query('SELECT 1 AS I, CAST(1.5 AS DECFLOAT(34)) AS D, TRUE AS B FROM SYSIBM.SYSDUMMY1');
      assert.deepEqual(r.rows, [{ I: 1, D: '1.5', B: true }]);
    });
    await check('BOOLEAN includes exactly TRUE, FALSE and NULL', async (c) => {
      const r = await c.query(`SELECT B FROM ${table} ORDER BY ID`);
      assert.equal(r.rowCount, 3);
      assert.deepEqual(r.rows, [{ B: true }, { B: false }, { B: null }]);
    });
    await check('SELECT * keeps every column and every row', async (c) => {
      const r = await c.query(`SELECT * FROM ${table} ORDER BY ID`);
      assert.equal(r.rowCount, 3);
      assert.deepEqual(r.columns.map((col) => col.name), ['ID', 'B', 'TS0', 'TS12', 'CL', 'BIG', 'X']);
      assert.equal(r.rows[0].CL, 'clob text');
      assert.equal(r.rows[0].BIG, '9223372036854775807');
      assert.equal(r.rows[2].X, null);
    });
    await check('XML retains NULL rows', async (c) => {
      const r = await c.query(`SELECT ID, X FROM ${table} ORDER BY ID`);
      assert.deepEqual(r.rows, [{ ID: 1, X: '<a>1</a>' }, { ID: 2, X: null }, { ID: 3, X: null }]);
    });
    await check('BIGINT reads and string/bigint parameters are lossless', async (c) => {
      assert.equal((await c.query(`SELECT BIG FROM ${table} WHERE ID=1`)).rows[0].BIG, '9223372036854775807');
      for (const value of ['9223372036854775807', 9223372036854775807n]) {
        assert.deepEqual((await c.query(`SELECT ID FROM ${table} WHERE BIG=?`, [value])).rows, [{ ID: 1 }]);
      }
    });
    await check('TIMESTAMP(0) and TIMESTAMP(12) retain their precision', async (c) => {
      assert.equal((await c.query(`SELECT TS0 FROM ${table} WHERE ID=1`)).rows[0].TS0, '2024-02-29-23.59.59');
      assert.equal((await c.query(`SELECT TS12 FROM ${table} WHERE ID=1`)).rows[0].TS12, '2024-02-29-23.59.59.123456789012');
    });
    await check('CLOB and catalog LOBs fetch successfully', async (c) => {
      assert.deepEqual((await c.query(`SELECT CL FROM ${table} ORDER BY ID`)).rows, [{ CL: 'clob text' }, { CL: null }, { CL: null }]);
      const r = await c.query(`SELECT COLNAME, "DEFAULT" FROM SYSCAT.COLUMNS WHERE TABNAME='${table}' ORDER BY COLNO`);
      assert.equal(r.rowCount, 7);
      assert.equal((await c.query(`SELECT XMLSERIALIZE(X AS CLOB(1K)) AS C FROM ${table} WHERE ID=1`)).rows[0].C, '<a>1</a>');
      const view = await c.query('SELECT TEXT FROM SYSCAT.VIEWS WHERE TEXT IS NOT NULL FETCH FIRST 1 ROW ONLY');
      assert.equal(view.rowCount, 1);
      assert.equal(typeof view.rows[0].TEXT, 'string');
    });
    await check('BLOB and DBCLOB materialize their values', async (c) => {
      const r = await c.query("SELECT BLOB(X'00FF12') AS B, DBCLOB('世界') AS D FROM SYSIBM.SYSDUMMY1");
      assert.deepEqual(r.rows[0].B, Buffer.from([0, 255, 18]));
      assert.equal(r.rows[0].D, '世界');
    });
    await check('multiple large UTF-8 CLOBs survive DRDA continuation blocks', async (c) => {
      const r = await c.query("SELECT ID, REPEAT(CLOB('héllo'),20000) AS C FROM (VALUES 1,2,3) AS V(ID) ORDER BY ID");
      assert.equal(r.rowCount, 3);
      for (let index = 0; index < 3; index++) {
        assert.equal(r.rows[index].ID, index + 1);
        assert.equal(r.rows[index].C, 'héllo'.repeat(20000));
      }
    });
    await check('BOOLEAN, CLOB and BLOB parameter descriptors are complete', async (c) => {
      const r = await c.query('SELECT CAST(? AS BOOLEAN) AS B, CAST(? AS CLOB(1K)) AS C, CAST(? AS BLOB(1K)) AS L FROM SYSIBM.SYSDUMMY1', [true, 'héllo', Buffer.from([0, 255, 18])]);
      assert.equal(r.rows[0].B, true);
      assert.equal(r.rows[0].C, 'héllo');
      assert.deepEqual(r.rows[0].L, Buffer.from([0, 255, 18]));
    });
    await check('Date parameters and GRAPHIC space padding', async (c) => {
      const r = await c.query('SELECT CAST(? AS TIMESTAMP) AS T, CAST(? AS GRAPHIC(4)) AS G FROM SYSIBM.SYSDUMMY1', [new Date('2024-02-29T23:59:59.123Z'), 'hi']);
      assert.equal(r.rows[0].T, '2024-02-29-23.59.59.123000');
      assert.equal(r.rows[0].G, 'hi  ');
    });
    await check('CALL works directly, prepared and within a transaction', async (c) => {
      const sql = `CALL SYSPROC.ADMIN_CMD('RUNSTATS ON TABLE DB2INST1.${table}')`;
      await c.query(sql);
      const statement = await c.prepare(sql);
      try { await statement.execute(); } finally { await statement.close(); }
      const transaction = await c.beginTransaction();
      try { await transaction.query(sql); } finally { await transaction.rollback(); }
    });
    await check('CALL returns multiple result sets and OUT parameters', async (c) => {
      const procedure = `${table}_P`;
      await c.query(`CREATE PROCEDURE ${procedure}(IN N INTEGER, OUT O BIGINT)
        LANGUAGE SQL DYNAMIC RESULT SETS 3 BEGIN
        DECLARE C1 CURSOR WITH RETURN TO CLIENT FOR SELECT N AS V, CAST('héllo' AS VARCHAR(20)) AS S FROM SYSIBM.SYSDUMMY1;
        DECLARE C2 CURSOR WITH RETURN TO CLIENT FOR SELECT B FROM ${table} ORDER BY ID;
        DECLARE C3 CURSOR WITH RETURN TO CLIENT FOR SELECT CL, X FROM ${table} ORDER BY ID;
        SET O=9223372036854775807; OPEN C1; OPEN C2; OPEN C3; END`);
      try {
        const routine = await c.query(`SELECT TEXT FROM SYSCAT.ROUTINES WHERE ROUTINENAME='${procedure}'`);
        assert.equal(routine.rowCount, 1);
        assert.match(routine.rows[0].TEXT, /DECLARE C1/i);
        const sql = `CALL ${procedure}(?, ?)`;
        const verify = (result: any) => {
          assert.deepEqual(result.rows, [{ V: 7, S: 'héllo' }]);
          assert.equal(result.resultSets.length, 3);
          assert.deepEqual(result.resultSets[1].rows, [{ B: true }, { B: false }, { B: null }]);
          assert.deepEqual(result.resultSets[2].rows, [{ CL: 'clob text', X: '<a>1</a>' }, { CL: null, X: null }, { CL: null, X: null }]);
          assert.deepEqual(result.outputParameters, ['9223372036854775807']);
        };
        verify(await c.query(sql, [7, null]));
        const statement = await c.prepare(sql);
        try { verify(await statement.execute([7, null])); } finally { await statement.close(); }
        const transaction = await c.beginTransaction();
        try { verify(await transaction.query(sql, [7, null])); } finally { await transaction.rollback(); }
        const database = await db2.open(config);
        try { verify(await database.queryResult(sql, [7, null])); } finally { await database.close(); }
      } finally { await c.query(`DROP PROCEDURE ${procedure}`); }
    });
    await check('native bigint, Date and binary parameters do not abort', async () => {
      const native = new db2.NativeClient(config);
      await native.connect();
      try {
        assert.equal((await native.query('VALUES CAST(? AS BIGINT)', [1n])).rows[0]['1'], 1);
        for (const bytes of [new Uint8Array([0, 255, 18]), new Uint8Array([0, 255, 18]).buffer]) {
          assert.deepEqual((await native.query('VALUES CAST(? AS BLOB(1K))', [bytes])).rows[0]['1'], Buffer.from([0, 255, 18]));
        }
        assert.equal((await native.query('VALUES CAST(? AS TIMESTAMP)', [new Date('2024-02-29T23:59:59.123Z')])).rows[0]['1'], '2024-02-29-23.59.59.123000');
        await assert.rejects(async () => native.query('VALUES CAST(? AS INTEGER)', [{ value: 1n }]), /Unsupported object parameter/);
      } finally { await native.close(); }
    });
    await t.test('queryTimeout cancels the server activity', async () => {
      const client = new Client({ ...config, queryTimeout: 300 });
      const monitor = new Client(config);
      await client.connect();
      await monitor.connect();
      try {
        const handle = Object.values((await client.query('VALUES MON_GET_APPLICATION_HANDLE()')).rows[0])[0];
        await assert.rejects(client.query('WITH N(I) AS (VALUES 1 UNION ALL SELECT I+1 FROM N WHERE I<100000000) SELECT SUM(BIGINT(I)) FROM N'), /server activity was cancelled/);
        const active = await monitor.query(`SELECT COUNT(*) AS C FROM TABLE(MON_GET_ACTIVITY(${handle}, -2))`);
        assert.equal(active.rows[0].C, 0);
      } finally { await client.close(); await monitor.close(); }
    });
    await check('cancel interrupts a query and preserves the connection', async (c) => {
      const running = c.query('WITH N(I) AS (VALUES 1 UNION ALL SELECT I+1 FROM N WHERE I<100000000) SELECT SUM(BIGINT(I)) FROM N').then(() => null, (error) => error);
      await delay(200);
      assert.equal(await c.cancel(), true);
      const error = await running;
      assert.equal(error?.sqlstate, '57014');
      assert.equal((await c.query('VALUES 1')).rows[0]['1'], 1);
    });
    await t.test('CALL works through the compatibility layer', async () => {
      const database = await db2.open(config);
      try { await database.query(`CALL SYSPROC.ADMIN_CMD('RUNSTATS ON TABLE DB2INST1.${table}')`); }
      finally { await database.close(); }
    });
    await t.test('currentSchema is applied and serverInfo names the product', async () => {
      const c = new Client({ ...config, currentSchema: 'SYSCAT' });
      await c.connect();
      try {
        assert.equal(String(Object.values((await c.query('VALUES CURRENT SCHEMA')).rows[0])[0]).trimEnd(), 'SYSCAT');
        assert.match((await c.serverInfo()).productName, /DB2/i);
      } finally { await c.close(); }
    });
    await t.test('LUW rejects unsupported type definitions explicitly', async () => {
      for (const typeDefinitionName of ['QTDSQLASC', 'QTDSQL370', 'QTDSQL400', 'none']) {
        const c = new Client({ ...config, typeDefinitionName });
        await assert.rejects(c.connect(), /typeDefinitionName.*not supported for Db2 LUW/);
      }
      const c = new Client({ ...config, typeDefinitionName: 'QTDSQLX86' });
      await c.connect();
      await c.close();
    });
  } finally {
    const c = new Client(config);
    await c.connect();
    try { await c.query(`DROP TABLE ${table}`); } finally { await c.close(); }
  }
});
