/**
 * CRUD Benchmark
 *
 * Tests all basic Create / Read / Update / Delete operations at scale,
 * measuring throughput for each operation type individually.
 *
 * Run:
 *   npm run crud
 *   CRUD_ROWS=50000 npm run crud
 */

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
  formatRate,
  type BenchResult,
} from "./helpers.js";

const TOTAL_ROWS = Number(process.env.CRUD_ROWS || 10_000);
const BATCH_SIZE = Number(process.env.CRUD_BATCH_SIZE || 2_000);
const TABLE = uniqueTable("CRUD");

async function main() {
  const client = createClient();
  const results: BenchResult[] = [];

  try {
    await client.connect();
    const info = await client.serverInfo();
    console.log(`Connected to ${info.productName} ${info.serverRelease}`);
    console.log(
      `CRUD benchmark — ${TOTAL_ROWS.toLocaleString()} rows, batch=${BATCH_SIZE.toLocaleString()}`,
    );

    await safeDropTable(client, TABLE);
    await client.query(`
      CREATE TABLE ${TABLE} (
        id        INTEGER NOT NULL PRIMARY KEY,
        name      VARCHAR(100) NOT NULL,
        email     VARCHAR(150),
        age       INTEGER,
        salary    DECIMAL(12,2),
        status    SMALLINT DEFAULT 1,
        bio       VARCHAR(500),
        created   TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
        updated   TIMESTAMP DEFAULT CURRENT_TIMESTAMP
      )
    `);

    // ── INSERT ──────────────────────────────────────────────────────────
    section("INSERT — batch prepared statement");

    const insertResult = await runBench("Batch INSERT", async () => {
      let inserted = 0;

      while (inserted < TOTAL_ROWS) {
        const batchEnd = Math.min(inserted + BATCH_SIZE, TOTAL_ROWS);
        const tx = await client.beginTransaction();
        const stmt = await tx.prepare(
          `INSERT INTO ${TABLE} (id, name, email, age, salary, status, bio) VALUES (?, ?, ?, ?, ?, ?, ?)`,
        );

        const rows: any[][] = [];
        for (let i = inserted; i < batchEnd; i++) {
          const id = i + 1;
          rows.push([
            id,
            `User_${id}_${randomString(8)}`,
            `user${id}@example.com`,
            randomInt(18, 80),
            randomInt(30000, 200000) + randomInt(0, 99) / 100,
            randomInt(0, 3),
            randomString(randomInt(50, 200)),
          ]);
        }

        await stmt.executeBatch(rows);
        await stmt.close();
        await tx.commit();

        inserted = batchEnd;
        process.stdout.write(
          `\r  inserted ${inserted.toLocaleString()} / ${TOTAL_ROWS.toLocaleString()} (${formatRate(inserted, performance.now())} rows/sec)`,
        );
      }
      console.log();
      return { rowsAffected: TOTAL_ROWS };
    });
    results.push(insertResult);
    printResult(insertResult);

    // ── SELECT — full table scan ────────────────────────────────────────
    section("SELECT — full table scan");

    const selectAllResult = await runBench("SELECT * (all rows)", async () => {
      const res = await client.query(`SELECT * FROM ${TABLE}`);
      return { rowsAffected: res.rows.length };
    });
    results.push(selectAllResult);
    printResult(selectAllResult);

    // ── SELECT — point lookups (by primary key) ─────────────────────────
    section("SELECT — point lookups (prepared)");

    const lookupCount = Math.min(5000, TOTAL_ROWS);
    const lookupResult = await runBench(
      `Point lookups x${lookupCount.toLocaleString()}`,
      async () => {
        const stmt = await client.prepare(
          `SELECT id, name, email, salary FROM ${TABLE} WHERE id = ?`,
        );
        for (let i = 0; i < lookupCount; i++) {
          await stmt.execute([randomInt(1, TOTAL_ROWS)]);
        }
        await stmt.close();
        return { rowsAffected: lookupCount };
      },
    );
    results.push(lookupResult);
    printResult(lookupResult);

    // ── SELECT — range scan with filtering ──────────────────────────────
    section("SELECT — range scan with filter");

    const rangeResult = await runBench(
      "Range scan (age BETWEEN, status filter)",
      async () => {
        const res = await client.query(
          `SELECT id, name, salary FROM ${TABLE} WHERE age BETWEEN ? AND ? AND status = ?`,
          [25, 45, 1],
        );
        return { rowsAffected: res.rows.length };
      },
    );
    results.push(rangeResult);
    printResult(rangeResult);

    // ── SELECT — aggregations ───────────────────────────────────────────
    section("SELECT — aggregations");

    const aggResult = await runBench(
      "Aggregations (COUNT, SUM, AVG, MIN, MAX)",
      async () => {
        const res = await client.query(`
        SELECT
          COUNT(*) AS cnt,
          SUM(salary) AS total_salary,
          AVG(salary) AS avg_salary,
          MIN(salary) AS min_salary,
          MAX(salary) AS max_salary,
          AVG(age) AS avg_age
        FROM ${TABLE}
      `);
        return { rowsAffected: 1, extra: res.rows[0] };
      },
    );
    results.push(aggResult);
    printResult(aggResult);

    // ── SELECT — GROUP BY ───────────────────────────────────────────────
    section("SELECT — GROUP BY");

    const groupResult = await runBench("GROUP BY status", async () => {
      const res = await client.query(`
        SELECT status, COUNT(*) AS cnt, AVG(salary) AS avg_sal
        FROM ${TABLE}
        GROUP BY status
        ORDER BY status
      `);
      return { rowsAffected: res.rows.length };
    });
    results.push(groupResult);
    printResult(groupResult);

    // ── UPDATE — bulk update ────────────────────────────────────────────
    section("UPDATE — bulk update all rows");

    const updateAllResult = await runBench(
      "UPDATE all rows (salary += 1000)",
      async () => {
        const res = await client.query(
          `UPDATE ${TABLE} SET salary = salary + 1000, updated = CURRENT_TIMESTAMP`,
        );
        return { rowsAffected: res.rowCount };
      },
    );
    results.push(updateAllResult);
    printResult(updateAllResult);

    // ── UPDATE — conditional update ─────────────────────────────────────
    section("UPDATE — conditional (WHERE clause)");

    const updateCondResult = await runBench(
      "UPDATE WHERE age > 60",
      async () => {
        const res = await client.query(
          `UPDATE ${TABLE} SET status = 0, bio = 'Retired' WHERE age > 60`,
        );
        return { rowsAffected: res.rowCount };
      },
    );
    results.push(updateCondResult);
    printResult(updateCondResult);

    // ── UPDATE — prepared statement single-row updates ──────────────────
    section("UPDATE — prepared single-row updates");

    const singleUpdateCount = Math.min(500, TOTAL_ROWS);
    const updateSingleResult = await runBench(
      `Single-row UPDATE x${singleUpdateCount.toLocaleString()}`,
      async () => {
        const tx = await client.beginTransaction();
        const txStmt = await tx.prepare(
          `UPDATE ${TABLE} SET name = ?, email = ? WHERE id = ?`,
        );
        for (let i = 0; i < singleUpdateCount; i++) {
          const id = randomInt(1, TOTAL_ROWS);
          await txStmt.execute([
            `Updated_${randomString(6)}`,
            `updated${id}@test.com`,
            id,
          ]);
        }
        await txStmt.close();
        await tx.commit();
        return { rowsAffected: singleUpdateCount };
      },
    );
    results.push(updateSingleResult);
    printResult(updateSingleResult);

    // ── DELETE — partial delete ─────────────────────────────────────────
    section("DELETE — partial (WHERE status = 0)");

    const deletePartialResult = await runBench(
      "DELETE WHERE status = 0",
      async () => {
        const res = await client.query(`DELETE FROM ${TABLE} WHERE status = 0`);
        return { rowsAffected: res.rowCount };
      },
    );
    results.push(deletePartialResult);
    printResult(deletePartialResult);

    // ── Verify remaining rows ───────────────────────────────────────────
    const remaining = await client.query(
      `SELECT COUNT(*) AS cnt FROM ${TABLE}`,
    );
    console.log(
      `\nRows remaining after partial delete: ${remaining.rows[0].CNT ?? remaining.rows[0].cnt ?? Object.values(remaining.rows[0])[0]}`,
    );

    // ── DELETE — delete all remaining rows ──────────────────────────────
    section("DELETE — purge remaining rows");

    const deleteAllResult = await runBench(
      "DELETE all remaining rows",
      async () => {
        const res = await client.query(`DELETE FROM ${TABLE}`);
        return { rowsAffected: res.rowCount };
      },
    );
    results.push(deleteAllResult);
    printResult(deleteAllResult);

    // ── Summary ─────────────────────────────────────────────────────────
    printSummary(results);
  } finally {
    await safeDropTable(client, TABLE);
    await client.close();
  }
}

main().catch((err) => {
  console.error("CRUD benchmark failed:", err);
  process.exit(1);
});
