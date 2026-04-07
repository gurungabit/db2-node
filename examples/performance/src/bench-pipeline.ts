/**
 * Data Pipeline Benchmark
 *
 * Simulates real-world data pipeline scenarios:
 *   - Rapid sequential fetches
 *   - Concurrent query pressure
 *   - Streaming-style pagination
 *   - ETL: read → transform → write
 *   - High-frequency prepared statement reuse
 *
 * Run:
 *   npm run pipeline
 */

import { Client, Pool } from "@gurungabit/db2-node";
import {
  createClient,
  createPool,
  uniqueTable,
  safeDropTable,
  safeDropTablePool,
  section,
  runBench,
  printResult,
  printSummary,
  randomString,
  randomInt,
  type BenchResult,
} from "./helpers.js";

const PIPELINE_ROWS = Number(process.env.PIPELINE_ROWS || 50_000);

async function main() {
  const client = createClient();
  const results: BenchResult[] = [];

  try {
    await client.connect();
    const info = await client.serverInfo();
    console.log(`Connected to ${info.productName} ${info.serverRelease}`);
    console.log(`Pipeline benchmark — ${PIPELINE_ROWS.toLocaleString()} rows\n`);

    // Seed data
    const sourceTable = uniqueTable("PIPE_SRC");
    await safeDropTable(client, sourceTable);
    await client.query(`
      CREATE TABLE ${sourceTable} (
        id      INTEGER NOT NULL,
        region  VARCHAR(20) NOT NULL,
        amount  DECIMAL(12,2) NOT NULL,
        ts      TIMESTAMP DEFAULT CURRENT_TIMESTAMP
      )
    `);

    console.log("  Seeding source table...");
    const regions = ["US-EAST", "US-WEST", "EU-WEST", "EU-EAST", "APAC", "LATAM"];
    let inserted = 0;
    while (inserted < PIPELINE_ROWS) {
      const batchEnd = Math.min(inserted + 5_000, PIPELINE_ROWS);
      const tx = await client.beginTransaction();
      const stmt = await tx.prepare(
        `INSERT INTO ${sourceTable} (id, region, amount) VALUES (?, ?, ?)`,
      );
      const rows: any[][] = [];
      for (let i = inserted; i < batchEnd; i++) {
        rows.push([i + 1, regions[i % regions.length], randomInt(100, 99999) / 100]);
      }
      await stmt.executeBatch(rows);
      await stmt.close();
      await tx.commit();
      inserted = batchEnd;
    }
    console.log("  Seeded.\n");

    // ── Rapid sequential fetches ────────────────────────────────────────
    section("Rapid sequential fetches (100 queries, no pause)");

    const rapidResult = await runBench("100 sequential queries", async () => {
      let totalRows = 0;
      for (let i = 0; i < 100; i++) {
        const res = await client.query(
          `SELECT id, region, amount FROM ${sourceTable} WHERE id BETWEEN ? AND ?`,
          [i * 500 + 1, (i + 1) * 500],
        );
        totalRows += res.rows.length;
      }
      return { rowsAffected: totalRows };
    });
    results.push(rapidResult);
    printResult(rapidResult);

    // ── Concurrent queries via connection pool ──────────────────────────
    section("Concurrent queries via Pool (10 parallel)");

    const pool = createPool({ maxConnections: 10, minConnections: 5 });

    const concurrentResult = await runBench("50 queries x 10 concurrent", async () => {
      let totalRows = 0;

      for (let batch = 0; batch < 5; batch++) {
        const promises = Array.from({ length: 10 }, (_, i) => {
          const offset = (batch * 10 + i) * 1000;
          return pool.query(
            `SELECT id, region, amount FROM ${sourceTable} WHERE id BETWEEN ? AND ?`,
            [offset + 1, offset + 1000],
          );
        });
        const results = await Promise.all(promises);
        for (const r of results) totalRows += r.rows.length;
      }

      return {
        rowsAffected: totalRows,
        extra: {
          idle: await pool.idleCount(),
          active: await pool.activeCount(),
          total: await pool.totalCount(),
        },
      };
    });
    results.push(concurrentResult);
    printResult(concurrentResult);

    // ── Streaming-style pagination ──────────────────────────────────────
    section("Pagination — cursor-style (ORDER BY + OFFSET/FETCH)");

    const pageSize = 1000;
    const paginationResult = await runBench(
      `Paginate ${PIPELINE_ROWS.toLocaleString()} rows (page=${pageSize})`,
      async () => {
        let totalFetched = 0;
        let offset = 0;
        let pages = 0;

        while (true) {
          const res = await client.query(
            `SELECT id, region, amount FROM ${sourceTable} ORDER BY id OFFSET ? ROWS FETCH FIRST ? ROWS ONLY`,
            [offset, pageSize],
          );
          if (res.rows.length === 0) break;
          totalFetched += res.rows.length;
          offset += pageSize;
          pages++;
        }

        return { rowsAffected: totalFetched, extra: { pages } };
      },
    );
    results.push(paginationResult);
    printResult(paginationResult);

    // ── Keyset pagination (more efficient) ──────────────────────────────
    section("Pagination — keyset (WHERE id > last_id)");

    const keysetResult = await runBench(
      `Keyset paginate ${PIPELINE_ROWS.toLocaleString()} rows`,
      async () => {
        let totalFetched = 0;
        let lastId = 0;
        let pages = 0;

        const stmt = await client.prepare(
          `SELECT id, region, amount FROM ${sourceTable} WHERE id > ? ORDER BY id FETCH FIRST ${pageSize} ROWS ONLY`,
        );

        while (true) {
          const res = await stmt.execute([lastId]);
          if (res.rows.length === 0) break;
          totalFetched += res.rows.length;
          const lastRow = res.rows[res.rows.length - 1];
          lastId = Number(lastRow.ID ?? lastRow.id);
          pages++;
        }
        await stmt.close();

        return { rowsAffected: totalFetched, extra: { pages } };
      },
    );
    results.push(keysetResult);
    printResult(keysetResult);

    // ── ETL: Read → Transform → Write ───────────────────────────────────
    section("ETL pipeline — read, transform, write to target table");

    const targetTable = uniqueTable("PIPE_TGT");
    try {
      await safeDropTable(client, targetTable);
      await client.query(`
        CREATE TABLE ${targetTable} (
          region       VARCHAR(20) NOT NULL,
          total_amount DECIMAL(15,2) NOT NULL,
          avg_amount   DECIMAL(12,2) NOT NULL,
          row_count    INTEGER NOT NULL
        )
      `);

      const etlResult = await runBench("ETL aggregate + write", async () => {
        // Read: aggregate by region
        const agg = await client.query(`
          SELECT region, SUM(amount) AS total, AVG(amount) AS avg_amt, COUNT(*) AS cnt
          FROM ${sourceTable}
          GROUP BY region
        `);

        // Transform + Write
        const tx = await client.beginTransaction();
        const stmt = await tx.prepare(
          `INSERT INTO ${targetTable} (region, total_amount, avg_amount, row_count) VALUES (?, ?, ?, ?)`,
        );
        const rows = agg.rows.map((r: any) => [
          r.REGION ?? r.region,
          Number(r.TOTAL ?? r.total),
          Number(r.AVG_AMT ?? r.avg_amt),
          Number(r.CNT ?? r.cnt),
        ]);
        await stmt.executeBatch(rows);
        await stmt.close();
        await tx.commit();

        return { rowsAffected: agg.rows.length, extra: { regionsProcessed: agg.rows.length } };
      });
      results.push(etlResult);
      printResult(etlResult);
    } finally {
      await safeDropTable(client, targetTable);
    }

    // ── High-frequency prepared statement reuse ─────────────────────────
    section("Prepared statement reuse — 10,000 executions");

    const prepReuseResult = await runBench("10,000 prepared executions", async () => {
      const stmt = await client.prepare(
        `SELECT COUNT(*) AS cnt FROM ${sourceTable} WHERE region = ? AND amount > ?`,
      );

      for (let i = 0; i < 10_000; i++) {
        await stmt.execute([regions[i % regions.length], randomInt(1, 500)]);
      }
      await stmt.close();
      return { rowsAffected: 10_000 };
    });
    results.push(prepReuseResult);
    printResult(prepReuseResult);

    // ── Rapid fire: many small queries ───────────────────────────────────
    section("Rapid fire — 1,000 tiny queries (SELECT 1)");

    const rapidFireResult = await runBench("1,000 x SELECT 1", async () => {
      for (let i = 0; i < 1_000; i++) {
        await client.query("VALUES 1");
      }
      return { rowsAffected: 1_000 };
    });
    results.push(rapidFireResult);
    printResult(rapidFireResult);

    // ── Concurrent pool fire ────────────────────────────────────────────
    section("Pool rapid fire — 500 concurrent queries");

    const poolFireResult = await runBench("500 concurrent pool queries", async () => {
      const promises = Array.from({ length: 500 }, (_, i) =>
        pool.query(
          `SELECT id, region, amount FROM ${sourceTable} WHERE id = ?`,
          [randomInt(1, PIPELINE_ROWS)],
        ),
      );
      const allResults = await Promise.all(promises);
      return {
        rowsAffected: allResults.reduce((sum, r) => sum + r.rows.length, 0),
        extra: {
          idle: await pool.idleCount(),
          active: await pool.activeCount(),
        },
      };
    });
    results.push(poolFireResult);
    printResult(poolFireResult);

    printSummary(results);

    // Cleanup
    await safeDropTable(client, sourceTable);
    await pool.close();
  } finally {
    await client.close();
  }
}

main().catch((err) => {
  console.error("Pipeline benchmark failed:", err);
  process.exit(1);
});
