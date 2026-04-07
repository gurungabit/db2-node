/**
 * Edge Cases Benchmark
 *
 * Tests unusual, extreme, and boundary conditions:
 *   - Empty result sets
 *   - Single-row tables
 *   - Maximum column name lengths
 *   - Unicode / special characters
 *   - Very long SQL statements
 *   - Rapid connect/disconnect
 *   - Transaction isolation
 *   - Duplicate key handling
 *   - Boundary numeric values
 *   - Empty strings vs NULLs
 *
 * Run:
 *   npm run edge
 */

import { Client } from "@gurungabit/db2-node";
import {
  createClient,
  uniqueTable,
  safeDropTable,
  section,
  runBench,
  printResult,
  printSummary,
  randomString,
  randomInt,
  connectionConfig,
  type BenchResult,
} from "./helpers.js";

async function main() {
  const client = createClient();
  const results: BenchResult[] = [];

  try {
    await client.connect();
    const info = await client.serverInfo();
    console.log(`Connected to ${info.productName} ${info.serverRelease}`);
    console.log("Edge Cases benchmark\n");

    // ── Empty result set ────────────────────────────────────────────────
    section("Empty result set queries");

    const emptyResult = await runBench("1,000 queries returning 0 rows", async () => {
      for (let i = 0; i < 1_000; i++) {
        await client.query("SELECT 1 FROM SYSIBM.SYSDUMMY1 WHERE 1 = 0");
      }
      return { rowsAffected: 0, extra: { queriesRun: 1_000 } };
    });
    results.push(emptyResult);
    printResult(emptyResult);

    // ── Single-row queries ──────────────────────────────────────────────
    section("Single-row queries (SYSDUMMY1)");

    const singleRowResult = await runBench("1,000 single-row queries", async () => {
      for (let i = 0; i < 1_000; i++) {
        await client.query("VALUES (CURRENT_TIMESTAMP, CURRENT_DATE, CURRENT_TIME)");
      }
      return { rowsAffected: 1_000 };
    });
    results.push(singleRowResult);
    printResult(singleRowResult);

    // ── Unicode / special characters ────────────────────────────────────
    section("Unicode and special characters");

    const unicodeTable = uniqueTable("UNICODE");
    try {
      await safeDropTable(client, unicodeTable);
      await client.query(`
        CREATE TABLE ${unicodeTable} (
          id   INTEGER NOT NULL,
          text VARCHAR(500)
        )
      `);

      const unicodeStrings = [
        "Hello World — basic ASCII",
        "日本語テスト — Japanese",
        "Привет мир — Russian",
        "مرحبا بالعالم — Arabic",
        "🎉🚀🔥💯 — Emoji",
        "Ñoño café résumé naïve — Accented Latin",
        "∑∏∫∂√∞≈≠ — Math symbols",
        "Tab\there\tnewline\ntest",
        "Quote's \"test\" and \\backslash\\",
        "NULL\x00embedded", // null byte
        "", // empty string
        " ", // single space
        "   leading and trailing spaces   ",
        "a".repeat(490), // near max VARCHAR length
      ];

      const unicodeInsertResult = await runBench(
        `INSERT ${unicodeStrings.length} unicode rows`,
        async () => {
          const tx = await client.beginTransaction();
          const stmt = await tx.prepare(`INSERT INTO ${unicodeTable} (id, text) VALUES (?, ?)`);
          const rows = unicodeStrings.map((s, i) => [i + 1, s]);
          await stmt.executeBatch(rows);
          await stmt.close();
          await tx.commit();
          return { rowsAffected: unicodeStrings.length };
        },
      );
      results.push(unicodeInsertResult);
      printResult(unicodeInsertResult);

      const unicodeReadResult = await runBench("READ unicode rows", async () => {
        const res = await client.query(`SELECT id, text FROM ${unicodeTable} ORDER BY id`);
        return { rowsAffected: res.rows.length };
      });
      results.push(unicodeReadResult);
      printResult(unicodeReadResult);

      // Search unicode data
      const unicodeSearchResult = await runBench("LIKE search unicode data", async () => {
        const res = await client.query(
          `SELECT id, text FROM ${unicodeTable} WHERE text LIKE ?`,
          ["%Japanese%"],
        );
        return { rowsAffected: res.rows.length };
      });
      results.push(unicodeSearchResult);
      printResult(unicodeSearchResult);
    } finally {
      await safeDropTable(client, unicodeTable);
    }

    // ── Boundary numeric values ─────────────────────────────────────────
    section("Boundary numeric values");

    const numTable = uniqueTable("NUMS");
    try {
      await safeDropTable(client, numTable);
      await client.query(`
        CREATE TABLE ${numTable} (
          id        INTEGER NOT NULL,
          small_val SMALLINT,
          int_val   INTEGER,
          big_val   BIGINT,
          dec_val   DECIMAL(31,10),
          real_val  REAL,
          dbl_val   DOUBLE
        )
      `);

      const boundaryResult = await runBench("INSERT boundary numeric values", async () => {
        const tx = await client.beginTransaction();
        const stmt = await tx.prepare(
          `INSERT INTO ${numTable} VALUES (?, ?, ?, ?, ?, ?, ?)`,
        );

        // Use individual inserts for boundary values since types need careful handling
        // BIGINT passed as string to avoid JS Number precision issues
        await stmt.execute([1, 0, 0, 0, "0", 0, 0]);                                   // zeros
        await stmt.execute([2, 32767, 2147483647, 2147483647, "99999999999999999999.9999999999", 3.4e38, 1.7e308]); // large values
        await stmt.execute([3, -32768, -2147483648, -2147483648, "-99999999999999999999.9999999999", -3.4e38, -1.7e308]); // min
        await stmt.execute([4, 1, 1, 1, "0.0000000001", 1.17549435e-38, 2.2250738585072014e-308]); // smallest positive
        await stmt.execute([5, -1, -1, -1, "-0.0000000001", -1.17549435e-38, -2.2250738585072014e-308]); // smallest negative
        await stmt.execute([6, null, null, null, null, null, null]);                     // all nulls
        await stmt.close();
        await tx.commit();
        return { rowsAffected: 6 };
      });
      results.push(boundaryResult);
      printResult(boundaryResult);

      const numReadResult = await runBench("READ boundary numerics", async () => {
        const res = await client.query(`SELECT * FROM ${numTable} ORDER BY id`);
        console.log("\n  Sample boundary rows:");
        for (const row of res.rows) {
          const id = row.ID ?? row.id;
          const big = row.BIG_VAL ?? row.big_val ?? "NULL";
          const dec = row.DEC_VAL ?? row.dec_val ?? "NULL";
          console.log(`    id=${id}  big=${big}  dec=${dec}`);
        }
        return { rowsAffected: res.rows.length };
      });
      results.push(numReadResult);
      printResult(numReadResult);
    } finally {
      await safeDropTable(client, numTable);
    }

    // ── Rapid connect / disconnect ──────────────────────────────────────
    section("Rapid connect/disconnect cycles");

    const connectCycles = 50;
    const connectResult = await runBench(
      `${connectCycles} connect/query/disconnect cycles`,
      async () => {
        for (let i = 0; i < connectCycles; i++) {
          const c = new Client(connectionConfig());
          await c.connect();
          await c.query("VALUES 1");
          await c.close();
        }
        return { rowsAffected: connectCycles };
      },
    );
    results.push(connectResult);
    printResult(connectResult);

    // ── Transaction edge cases ──────────────────────────────────────────
    section("Transaction edge cases");

    const txTable = uniqueTable("TX_EDGE");
    try {
      await safeDropTable(client, txTable);
      await client.query(`
        CREATE TABLE ${txTable} (id INTEGER NOT NULL PRIMARY KEY, val INTEGER)
      `);

      // Empty transaction commit
      const emptyTxResult = await runBench("100 empty transaction commits", async () => {
        for (let i = 0; i < 100; i++) {
          const tx = await client.beginTransaction();
          await tx.commit();
        }
        return { rowsAffected: 100 };
      });
      results.push(emptyTxResult);
      printResult(emptyTxResult);

      // Empty transaction rollback
      const emptyRollbackResult = await runBench("100 empty transaction rollbacks", async () => {
        for (let i = 0; i < 100; i++) {
          const tx = await client.beginTransaction();
          await tx.rollback();
        }
        return { rowsAffected: 100 };
      });
      results.push(emptyRollbackResult);
      printResult(emptyRollbackResult);

      // Insert then rollback then insert again
      const reinsertResult = await runBench("Insert → rollback → reinsert (20x)", async () => {
        for (let i = 0; i < 20; i++) {
          const tx1 = await client.beginTransaction();
          await tx1.query(`INSERT INTO ${txTable} (id, val) VALUES (?, ?)`, [i + 1, i]);
          await tx1.rollback();

          // Reinsert with same PK should succeed after rollback
          const tx2 = await client.beginTransaction();
          await tx2.query(`INSERT INTO ${txTable} (id, val) VALUES (?, ?)`, [i + 1, i * 10]);
          await tx2.commit();
        }
        return { rowsAffected: 20 };
      });
      results.push(reinsertResult);
      printResult(reinsertResult);
    } finally {
      await safeDropTable(client, txTable);
    }

    // ── Duplicate key error handling ────────────────────────────────────
    section("Duplicate key error handling");

    const dupTable = uniqueTable("DUP");
    try {
      await safeDropTable(client, dupTable);
      await client.query(`
        CREATE TABLE ${dupTable} (id INTEGER NOT NULL PRIMARY KEY, val VARCHAR(50))
      `);
      await client.query(`INSERT INTO ${dupTable} VALUES (1, 'original')`);

      const dupResult = await runBench("100 duplicate key errors (caught)", async () => {
        let errorCount = 0;
        for (let i = 0; i < 100; i++) {
          try {
            await client.query(`INSERT INTO ${dupTable} VALUES (1, 'dup_${i}')`);
          } catch {
            errorCount++;
          }
        }
        return { rowsAffected: 100, extra: { errorsHandled: errorCount } };
      });
      results.push(dupResult);
      printResult(dupResult);
    } finally {
      await safeDropTable(client, dupTable);
    }

    // ── Very long SQL ───────────────────────────────────────────────────
    section("Long SQL statements");

    const longSqlResult = await runBench("Query with 200 UNION ALLs", async () => {
      const parts: string[] = [];
      for (let i = 0; i < 200; i++) {
        parts.push(`SELECT ${i + 1} AS id, '${randomString(20)}' AS val FROM SYSIBM.SYSDUMMY1`);
      }
      const sql = parts.join(" UNION ALL ");
      const res = await client.query(sql);
      return { rowsAffected: res.rows.length, extra: { sqlLength: sql.length } };
    });
    results.push(longSqlResult);
    printResult(longSqlResult);

    // ── Large IN clause ─────────────────────────────────────────────────
    section("Large IN clause");

    const inTable = uniqueTable("IN_CLAUSE");
    try {
      await safeDropTable(client, inTable);
      await client.query(`CREATE TABLE ${inTable} (id INTEGER NOT NULL, val VARCHAR(20))`);

      // Insert 1000 rows
      const tx = await client.beginTransaction();
      const stmt = await tx.prepare(`INSERT INTO ${inTable} (id, val) VALUES (?, ?)`);
      const rows: any[][] = [];
      for (let i = 0; i < 1_000; i++) {
        rows.push([i + 1, `val_${i}`]);
      }
      await stmt.executeBatch(rows);
      await stmt.close();
      await tx.commit();

      // Query with large parameterized IN
      const inCount = 500;
      const inResult = await runBench(`SELECT with ${inCount} params in IN()`, async () => {
        const ids = Array.from({ length: inCount }, (_, i) => randomInt(1, 1000));
        const placeholders = ids.map(() => "?").join(", ");
        const res = await client.query(
          `SELECT id, val FROM ${inTable} WHERE id IN (${placeholders})`,
          ids,
        );
        return { rowsAffected: res.rows.length };
      });
      results.push(inResult);
      printResult(inResult);
    } finally {
      await safeDropTable(client, inTable);
    }

    // ── Prepared statement reopen ────────────────────────────────────────
    section("Prepared statement open/close churn");

    const prepChurnResult = await runBench("500 prepare → execute → close cycles", async () => {
      for (let i = 0; i < 500; i++) {
        const stmt = await client.prepare("VALUES CAST(? AS INTEGER) + ?");
        await stmt.execute([i, i * 2]);
        await stmt.close();
      }
      return { rowsAffected: 500 };
    });
    results.push(prepChurnResult);
    printResult(prepChurnResult);

    // ── Metadata queries ────────────────────────────────────────────────
    section("Metadata / catalog queries");

    const metaResult = await runBench("Query SYSCAT.TABLES (catalog)", async () => {
      const res = await client.query(
        "SELECT TABNAME, TABSCHEMA, TYPE FROM SYSCAT.TABLES FETCH FIRST 100 ROWS ONLY",
      );
      return { rowsAffected: res.rows.length };
    });
    results.push(metaResult);
    printResult(metaResult);

    const colMetaResult = await runBench("Query SYSCAT.COLUMNS (catalog)", async () => {
      const res = await client.query(
        "SELECT TABNAME, COLNAME, TYPENAME, LENGTH FROM SYSCAT.COLUMNS FETCH FIRST 500 ROWS ONLY",
      );
      return { rowsAffected: res.rows.length };
    });
    results.push(colMetaResult);
    printResult(colMetaResult);

    printSummary(results);
  } finally {
    await client.close();
  }
}

main().catch((err) => {
  console.error("Edge cases benchmark failed:", err);
  process.exit(1);
});
