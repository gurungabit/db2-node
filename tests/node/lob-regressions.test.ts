import test from 'node:test';
import assert from 'node:assert/strict';
import { Client, getConfig } from './helpers';

const uniqueTable = (issue: number) => `LOB_I${issue}_${process.pid}_${process.hrtime.bigint()}`;

test('issue #19: CLOB and four-byte BLOB keep EXTDTA order across rows and fetches', { timeout: 120_000 }, async () => {
  const client = new Client({ ...getConfig(), fetchSize: 1 });
  await client.connect();
  try {
    for (const size of ['1K', '1M']) {
      const table = uniqueTable(19);
      await client.query(`CREATE TABLE ${table}(ID INTEGER NOT NULL, CL CLOB(${size}), BL BLOB(${size}))`);
      try {
        await client.query(`INSERT INTO ${table} VALUES
          (1, 'clob-one', BLOB(X'0001FEFF')),
          (2, 'clob text xxxxxxxxxxxxxxxxxxxx', BLOB(X'ABCD1234')),
          (3, NULL, NULL),
          (4, '', BLOB(X'')),
          (5, 'after null', BLOB(X'01020304'))`);
        const expected = [
          { CL: 'clob-one', BL: Buffer.from('0001feff', 'hex') },
          { CL: 'clob text xxxxxxxxxxxxxxxxxxxx', BL: Buffer.from('abcd1234', 'hex') },
          { CL: null, BL: null },
          { CL: '', BL: Buffer.alloc(0) },
          { CL: 'after null', BL: Buffer.from('01020304', 'hex') },
        ];
        for (const projection of ['CL, BL', 'BL, CL']) {
          const sql = `SELECT ${projection} FROM ${table} ORDER BY ID`;
          const result = await client.query(sql);
          assert.equal(result.rowCount, expected.length);
          assert.deepEqual(result.rows, expected);
          const statement = await client.prepare(sql);
          try { assert.deepEqual((await statement.execute()).rows, expected); }
          finally { await statement.close(); }
        }
      } finally { await client.query(`DROP TABLE ${table}`); }
    }
    assert.deepEqual((await client.query('VALUES 1')).rows, [{ '1': 1 }]);
  } finally { await client.close(); }
});

test('issue #20: 25-column rows retain DOUBLE, GRAPHIC and mixed large/NULL LOBs', { timeout: 120_000 }, async () => {
  const client = new Client({ ...getConfig(), fetchSize: 1 });
  await client.connect();
  const table = uniqueTable(20);
  const extraColumns = Array.from({ length: 18 }, (_, index) => `N${index + 1}`);
  try {
    await client.query(`CREATE TABLE ${table}(ID INTEGER NOT NULL, D DOUBLE, G GRAPHIC(3), CL CLOB(1M), DC DBCLOB(1M), BL BLOB(1M), X XML, ${extraColumns.map(name => `${name} INTEGER`).join(',')})`);
    try {
      await client.query(`INSERT INTO ${table} (ID,D,G,CL,DC,BL,X) VALUES
        (1,1.5,'abc','clob text xxxxxxxxxxxxxxxxxxxx','漢é',BLOB(X'0001FEFF'),XMLPARSE(DOCUMENT '<x>one</x>')),
        (2,2.5,'def',NULL,NULL,NULL,NULL),
        (3,3.5,'ghi',REPEAT(CLOB('z'),70000),'wide',BLOB(X'ABCD1234'),XMLPARSE(DOCUMENT '<x>three</x>')),
        (4,4.5,'jkl','', '',BLOB(X''),NULL),
        (5,5.5,'mno',NULL,NULL,NULL,NULL)`);
      const expected = ['abc', 'def', 'ghi', 'jkl', 'mno'].map((g, index) => ({
        ID: index + 1, D: index + 1.5, G: g,
        CL: ['clob text xxxxxxxxxxxxxxxxxxxx', null, 'z'.repeat(70000), '', null][index],
        DC: ['漢é', null, 'wide', '', null][index],
        BL: [Buffer.from('0001feff', 'hex'), null, Buffer.from('abcd1234', 'hex'), Buffer.alloc(0), null][index],
        X: ['<x>one</x>', null, '<x>three</x>', null, null][index],
        ...Object.fromEntries(extraColumns.map(name => [name, null])),
      }));
      for (const projection of ['D, CL', 'G, CL', 'CL, D, G', '*']) {
        const names = projection === '*' ? Object.keys(expected[0]) : projection.split(', ');
        const projected = expected.map(row => Object.fromEntries(names.map(name => [name, row[name as keyof typeof row]])));
        const sql = `SELECT ${projection} FROM ${table} WHERE ID >= ? ORDER BY ID`;
        const result = await client.query(sql, [1]);
        assert.equal(result.rowCount, expected.length);
        assert.equal(result.columns.length, names.length);
        assert.deepEqual(result.rows, projected);
        const statement = await client.prepare(sql);
        try {
          const prepared = await statement.execute([1]);
          assert.equal(prepared.rowCount, expected.length);
          assert.equal(prepared.columns.length, names.length);
          assert.deepEqual(prepared.rows, projected);
        } finally { await statement.close(); }
      }
    } finally { await client.query(`DROP TABLE ${table}`); }
    assert.deepEqual((await client.query('VALUES 1')).rows, [{ '1': 1 }]);
  } finally { await client.close(); }
});

test('issue #31: parameters bound to LOBs declared beyond 32767 bytes are written', { timeout: 120_000 }, async () => {
  const client = new Client(getConfig());
  await client.connect();
  const table = uniqueTable(31);
  const select = `SELECT ID, SMALL, BIG, BB, DB FROM ${table} ORDER BY ID`;
  try {
    await client.query(`CREATE TABLE ${table} (ID INTEGER NOT NULL PRIMARY KEY, SMALL CLOB(32767), BIG CLOB(32768), BB BLOB(1M), DB DBCLOB(32768))`);
    try {
      await client.query(`INSERT INTO ${table} (ID, SMALL, BIG) VALUES (1, 'old', 'old')`);
      for (const [sql, params] of [
        [`UPDATE ${table} SET SMALL = ? WHERE ID = 1`, ['new']],
        [`UPDATE ${table} SET BIG = ? WHERE ID = 1`, ['new']],
        [`UPDATE ${table} SET BB = ? WHERE ID = 1`, [Buffer.from([1, 2, 3])]],
        [`UPDATE ${table} SET DB = ? WHERE ID = 1`, ['漢é\u{1D11E}']],
      ] as const) {
        assert.equal((await client.query(sql, [...params])).rowCount, 1, sql);
      }
      assert.deepEqual((await client.query(select)).rows, [
        { ID: 1, SMALL: 'new', BIG: 'new', BB: Buffer.from([1, 2, 3]), DB: '漢é\u{1D11E}' },
      ]);

      assert.equal((await client.query(`UPDATE ${table} SET BB = CAST(? AS BLOB(1M)) WHERE ID = ?`, [Buffer.from([4, 5]), 1])).rowCount, 1);
      assert.equal((await client.query(`UPDATE ${table} SET BIG = CAST(? AS VARCHAR(100)) WHERE ID = 1`, ['cast'])).rowCount, 1);
      assert.deepEqual((await client.query(`SELECT BIG, BB FROM ${table} WHERE ID = 1`)).rows, [{ BIG: 'cast', BB: Buffer.from([4, 5]) }]);

      // Boundary-sized and genuinely large values, several LOB parameters in
      // one statement, followed by a non-LOB parameter.
      const big = 'é'.repeat(16_384);
      const blob = Buffer.from(Array.from({ length: 600_000 }, (_, index) => (index * 7) % 256));
      const graphic = '漢\u{1D11E}'.repeat(10_000);
      const update = `UPDATE ${table} SET SMALL = ?, BIG = ?, BB = ?, DB = ? WHERE ID = ?`;
      assert.equal((await client.query(update, ['small', big, blob, graphic, 1])).rowCount, 1);
      assert.deepEqual((await client.query(select)).rows, [{ ID: 1, SMALL: 'small', BIG: big, BB: blob, DB: graphic }]);
      await assert.rejects(client.query(`UPDATE ${table} SET BIG = ? WHERE ID = 1`, [`${big}x`]), { sqlstate: '22001' });
      assert.equal((await client.query(`SELECT LENGTH(BIG) AS L FROM ${table}`)).rows[0].L, 32_768);

      const statement = await client.prepare(update);
      try {
        assert.equal((await statement.execute(['', '', Buffer.alloc(0), null, 1])).rowCount, 1);
        assert.deepEqual((await client.query(select)).rows, [{ ID: 1, SMALL: '', BIG: '', BB: Buffer.alloc(0), DB: null }]);
      } finally { await statement.close(); }

      const insert = await client.prepare(`INSERT INTO ${table} (ID, BIG, BB, DB) VALUES (?, ?, ?, ?)`);
      try {
        assert.equal((await insert.executeBatch([[2, 'two', Buffer.from([2]), 'b'], [3, null, null, '']])).rowCount, 2);
      } finally { await insert.close(); }
      assert.deepEqual((await client.query(`SELECT ID, BIG, BB, DB FROM ${table} WHERE ID > 1 ORDER BY ID`)).rows, [
        { ID: 2, BIG: 'two', BB: Buffer.from([2]), DB: 'b' },
        { ID: 3, BIG: null, BB: null, DB: '' },
      ]);

      assert.deepEqual((await client.query(`SELECT ID FROM ${table} WHERE LENGTH(CAST(? AS CLOB(1M))) = ? ORDER BY ID`, [big, 32_768])).rows, [
        { ID: 1 }, { ID: 2 }, { ID: 3 },
      ]);
    } finally { await client.query(`DROP TABLE ${table}`); }
    assert.deepEqual((await client.query('VALUES 1')).rows, [{ '1': 1 }]);
  } finally { await client.close(); }
});

test('issue #31: empty LOB parameters do not shift later EXTDTA values', { timeout: 180_000 }, async () => {
  const client = new Client(getConfig());
  await client.connect();
  const table = uniqueTable(31);
  const routine = `${table}_P`;
  const select = `SELECT ID, C, X, B, G FROM ${table} ORDER BY ID`;
  let routineCreated = false;
  try {
    await client.query(`CREATE TABLE ${table} (ID INTEGER NOT NULL PRIMARY KEY, C CLOB(32768), X CLOB(2M), B BLOB(2M), G DBCLOB(1M))`);
    try {
      // Every empty/NULL/small/large order of the externalized columns;
      // empty and NULL values must stay distinct.
      const kinds = {
        empty: ['', '', Buffer.alloc(0), ''],
        null: [null, null, null, null],
        small: ['é', 'x€\u{1D11E}', Buffer.from([0, 255, 1]), '漢\u{1D11E}'],
        large: ['c'.repeat(32_768), 'x€'.repeat(20_000), Buffer.alloc(40_000, 3), '漢'.repeat(20_000)],
      } as const;
      const names = Object.keys(kinds) as (keyof typeof kinds)[];
      const combinations = names.flatMap(c => names.flatMap(x => names.flatMap(b => names.map(g =>
        [kinds[c][0], kinds[x][1], kinds[b][2], kinds[g][3]]))));
      let rows = combinations.map((values, index) => [index + 1, ...values]);
      const shift = (by: number) => rows.map(([id], index) => [id, ...combinations[(index + by) % combinations.length]]);

      const insert = await client.prepare(`INSERT INTO ${table} VALUES (?, ?, ?, ?, ?)`);
      try {
        assert.equal((await insert.executeBatch(rows)).rowCount, rows.length);
        // Reviewer shape: 10 rows of 33-60KB values with NULL and empty mixed in.
        const mixed = Array.from({ length: 10 }, (_, i) => [
          i + 1_000, i % 2 ? 'b' : null, i % 3 ? 'b€'.repeat(12_000) : null,
          i % 4 ? Buffer.alloc(33_000 + i, i) : Buffer.alloc(0), i % 5 ? '漢'.repeat(20_000) : null,
        ]);
        assert.equal((await insert.executeBatch(mixed)).rowCount, mixed.length);
        assert.deepEqual((await client.query(select, [], { rowMode: 'array' })).rows, [...rows, ...mixed]);
        await client.query(`DELETE FROM ${table} WHERE ID >= 1000`);
      } finally { await insert.close(); }

      // Direct and prepared single executions alternate per row.
      rows = shift(1);
      const updateSql = `UPDATE ${table} SET C = ?, X = ?, B = ?, G = ? WHERE ID = ?`;
      const update = await client.prepare(updateSql);
      try {
        for (const [index, [id, ...values]] of rows.entries()) {
          const result = index % 2 ? await update.execute([...values, id]) : await client.query(updateSql, [...values, id]);
          assert.equal(result.rowCount, 1);
        }
      } finally { await update.close(); }
      assert.deepEqual((await client.query(select, [], { rowMode: 'array' })).rows, rows);

      await client.query(`CREATE PROCEDURE ${routine} (IN P_ID INTEGER, IN P_C CLOB(32768), IN P_X CLOB(2M), IN P_B BLOB(2M), IN P_G DBCLOB(1M)) LANGUAGE SQL BEGIN UPDATE ${table} SET C = P_C, X = P_X, B = P_B, G = P_G WHERE ID = P_ID; END`);
      routineCreated = true;
      rows = shift(2);
      for (const [id, ...values] of rows) {
        await client.query(`CALL ${routine}(?, ?, ?, ?, ?)`, [id, ...values]);
      }
      assert.deepEqual((await client.query(select, [], { rowMode: 'array' })).rows, rows);
    } finally {
      if (routineCreated) await client.query(`DROP PROCEDURE ${routine}`);
      await client.query(`DROP TABLE ${table}`);
    }
    assert.deepEqual((await client.query('VALUES 1')).rows, [{ '1': 1 }]);
  } finally { await client.close(); }
});

test('issue #31: XML parameters beyond 32767 bytes are written', { timeout: 120_000 }, async () => {
  const client = new Client(getConfig());
  await client.connect();
  const table = uniqueTable(31);
  try {
    await client.query(`CREATE TABLE ${table} (ID INTEGER NOT NULL PRIMARY KEY, X XML, C CLOB(1M))`);
    try {
      const small = '<a>é</a>';
      const large = `<b>${'x€'.repeat(30_000)}</b>`;
      const select = `SELECT ID, XMLSERIALIZE(X AS CLOB(1M)) AS X FROM ${table} ORDER BY ID`;
      assert.equal((await client.query(`INSERT INTO ${table} (ID, X, C) VALUES (?, ?, ?)`, [1, large, 'c'.repeat(40_000)])).rowCount, 1);
      assert.equal((await client.query(`UPDATE ${table} SET X = ? WHERE ID = ?`, [large.replace('b>', 'c>').replace('</b', '</c'), 1])).rowCount, 1);
      const insert = await client.prepare(`INSERT INTO ${table} (ID, X) VALUES (?, ?)`);
      try {
        assert.equal((await insert.executeBatch([[2, small], [3, null], [4, large]])).rowCount, 3);
      } finally { await insert.close(); }
      assert.deepEqual((await client.query(select)).rows, [
        { ID: 1, X: `<c>${'x€'.repeat(30_000)}</c>` },
        { ID: 2, X: small },
        { ID: 3, X: null },
        { ID: 4, X: large },
      ]);
      assert.deepEqual((await client.query(`SELECT ID FROM ${table} WHERE XMLEXISTS('$d/b' PASSING CAST(? AS XML) AS "d") AND ID = ?`, [large, 4])).rows, [{ ID: 4 }]);
      await assert.rejects(client.query(`UPDATE ${table} SET X = ? WHERE ID = 1`, ['<a>']), { sqlstate: '2200M' });
    } finally { await client.query(`DROP TABLE ${table}`); }
    assert.deepEqual((await client.query('VALUES 1')).rows, [{ '1': 1 }]);
  } finally { await client.close(); }
});
