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
