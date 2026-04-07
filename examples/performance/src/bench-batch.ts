/**
 * Large Batch Operations Benchmark
 *
 * Tests massive insert and delete operations with various batch sizes,
 * commit intervals, and strategies to find optimal throughput.
 *
 * Run:
 *   npm run batch
 *   BATCH_TOTAL=500000 npm run batch
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
  formatRate,
  randomString,
  randomInt,
  type BenchResult,
} from "./helpers.js";

const TOTAL_ROWS = Number(process.env.BATCH_TOTAL || 100_000);

async function main() {
  const client = createClient();
  const results: BenchResult[] = [];

  try {
    await client.connect();
    const info = await client.serverInfo();
    console.log(`Connected to ${info.productName} ${info.serverRelease}`);
    console.log(`Batch benchmark — ${TOTAL_ROWS.toLocaleString()} rows\n`);

    // ── Test different batch sizes ──────────────────────────────────────
    section("INSERT — varying batch sizes (finding optimal)");

    const batchSizes = [100, 500, 1_000, 5_000, 10_000];

    for (const batchSize of batchSizes) {
      const table = uniqueTable(`BATCH_${batchSize}`);
      const rowCount = Math.min(TOTAL_ROWS, 50_000); // cap per test to keep total time reasonable

      try {
        await safeDropTable(client, table);
        await client.query(`
          CREATE TABLE ${table} (
            id      INTEGER NOT NULL,
            val     INTEGER NOT NULL,
            label   VARCHAR(100) NOT NULL,
            data    VARCHAR(200)
          )
        `);

        const r = await runBench(`batch_size=${batchSize.toLocaleString()}`, async () => {
          let inserted = 0;
          const commitEvery = Math.max(batchSize * 5, 10_000);

          let tx = await client.beginTransaction();
          let stmt = await tx.prepare(
            `INSERT INTO ${table} (id, val, label, data) VALUES (?, ?, ?, ?)`,
          );
          let sinceCommit = 0;

          while (inserted < rowCount) {
            const nextBatch = Math.min(batchSize, rowCount - inserted);
            const rows: any[][] = [];
            for (let j = 0; j < nextBatch; j++) {
              const id = inserted + j + 1;
              rows.push([id, randomInt(1, 999999), `Row_${id}`, randomString(50)]);
            }
            await stmt.executeBatch(rows);
            inserted += nextBatch;
            sinceCommit += nextBatch;

            if (sinceCommit >= commitEvery || inserted >= rowCount) {
              await stmt.close();
              await tx.commit();
              sinceCommit = 0;
              if (inserted < rowCount) {
                tx = await client.beginTransaction();
                stmt = await tx.prepare(
                  `INSERT INTO ${table} (id, val, label, data) VALUES (?, ?, ?, ?)`,
                );
              }
            }
          }

          return { rowsAffected: inserted };
        });
        results.push(r);
        printResult(r);
      } finally {
        await safeDropTable(client, table);
      }
    }

    // ── Large single-commit insert ──────────────────────────────────────
    section("INSERT — large single-commit (transaction stress)");

    const singleCommitTable = uniqueTable("SINGLE_COMMIT");
    const singleCommitRows = Math.min(TOTAL_ROWS, 50_000);

    try {
      await safeDropTable(client, singleCommitTable);
      await client.query(`
        CREATE TABLE ${singleCommitTable} (
          id INTEGER NOT NULL, val INTEGER NOT NULL, label VARCHAR(100) NOT NULL
        )
      `);

      const r = await runBench(
        `Single commit (${singleCommitRows.toLocaleString()} rows)`,
        async () => {
          const tx = await client.beginTransaction();
          const stmt = await tx.prepare(
            `INSERT INTO ${singleCommitTable} (id, val, label) VALUES (?, ?, ?)`,
          );

          let inserted = 0;
          const batchSize = 5_000;
          while (inserted < singleCommitRows) {
            const nextBatch = Math.min(batchSize, singleCommitRows - inserted);
            const rows: any[][] = [];
            for (let j = 0; j < nextBatch; j++) {
              const id = inserted + j + 1;
              rows.push([id, id * 3, `Single_${id}`]);
            }
            await stmt.executeBatch(rows);
            inserted += nextBatch;
          }
          await stmt.close();
          await tx.commit();
          return { rowsAffected: singleCommitRows };
        },
      );
      results.push(r);
      printResult(r);
    } finally {
      await safeDropTable(client, singleCommitTable);
    }

    // ── Large DELETE benchmarks ─────────────────────────────────────────
    section("DELETE — large deletes");

    const deleteTable = uniqueTable("DEL");
    const deleteRows = Math.min(TOTAL_ROWS, 100_000);

    try {
      // Populate
      await safeDropTable(client, deleteTable);
      await client.query(`
        CREATE TABLE ${deleteTable} (
          id INTEGER NOT NULL, bucket INTEGER NOT NULL, data VARCHAR(100)
        )
      `);

      console.log(`  Populating ${deleteRows.toLocaleString()} rows for delete tests...`);
      let inserted = 0;
      while (inserted < deleteRows) {
        const batchEnd = Math.min(inserted + 5_000, deleteRows);
        const tx = await client.beginTransaction();
        const stmt = await tx.prepare(
          `INSERT INTO ${deleteTable} (id, bucket, data) VALUES (?, ?, ?)`,
        );
        const rows: any[][] = [];
        for (let i = inserted; i < batchEnd; i++) {
          rows.push([i + 1, i % 10, randomString(40)]);
        }
        await stmt.executeBatch(rows);
        await stmt.close();
        await tx.commit();
        inserted = batchEnd;
      }
      console.log(`  Populated.\n`);

      // Delete by bucket (10% at a time)
      const delBucketResult = await runBench("DELETE by bucket (10% of data)", async () => {
        const res = await client.query(`DELETE FROM ${deleteTable} WHERE bucket = 0`);
        return { rowsAffected: res.rowCount };
      });
      results.push(delBucketResult);
      printResult(delBucketResult);

      // Delete with range
      const halfPoint = Math.floor(deleteRows / 2);
      const delRangeResult = await runBench("DELETE range (id > half)", async () => {
        const res = await client.query(`DELETE FROM ${deleteTable} WHERE id > ?`, [halfPoint]);
        return { rowsAffected: res.rowCount };
      });
      results.push(delRangeResult);
      printResult(delRangeResult);

      // Delete all remaining
      const delAllResult = await runBench("DELETE all remaining", async () => {
        const res = await client.query(`DELETE FROM ${deleteTable}`);
        return { rowsAffected: res.rowCount };
      });
      results.push(delAllResult);
      printResult(delAllResult);
    } finally {
      await safeDropTable(client, deleteTable);
    }

    // ── INSERT + immediate DELETE cycle ─────────────────────────────────
    section("INSERT then DELETE cycle (churn)");

    const churnTable = uniqueTable("CHURN");
    const churnCycles = 20;
    const churnBatch = 2_000;

    try {
      await safeDropTable(client, churnTable);
      await client.query(`
        CREATE TABLE ${churnTable} (id INTEGER NOT NULL, data VARCHAR(200))
      `);

      const churnResult = await runBench(
        `${churnCycles} insert/delete cycles x${churnBatch.toLocaleString()} rows`,
        async () => {
          let totalRows = 0;
          for (let cycle = 0; cycle < churnCycles; cycle++) {
            const tx = await client.beginTransaction();
            const stmt = await tx.prepare(
              `INSERT INTO ${churnTable} (id, data) VALUES (?, ?)`,
            );
            const rows: any[][] = [];
            for (let j = 0; j < churnBatch; j++) {
              rows.push([cycle * churnBatch + j + 1, randomString(100)]);
            }
            await stmt.executeBatch(rows);
            await stmt.close();
            await tx.commit();
            totalRows += churnBatch;

            // Delete everything
            await client.query(`DELETE FROM ${churnTable}`);
          }
          return { rowsAffected: totalRows };
        },
      );
      results.push(churnResult);
      printResult(churnResult);
    } finally {
      await safeDropTable(client, churnTable);
    }

    // ── Transaction rollback stress ─────────────────────────────────────
    section("ROLLBACK — large transaction rollback");

    const rollbackTable = uniqueTable("ROLLBACK");
    const rollbackRows = Math.min(20_000, TOTAL_ROWS);

    try {
      await safeDropTable(client, rollbackTable);
      await client.query(`
        CREATE TABLE ${rollbackTable} (id INTEGER NOT NULL, data VARCHAR(100))
      `);

      const rollbackResult = await runBench(
        `Rollback ${rollbackRows.toLocaleString()} row insert`,
        async () => {
          const tx = await client.beginTransaction();
          const stmt = await tx.prepare(
            `INSERT INTO ${rollbackTable} (id, data) VALUES (?, ?)`,
          );
          const rows: any[][] = [];
          for (let i = 0; i < rollbackRows; i++) {
            rows.push([i + 1, randomString(50)]);
          }
          await stmt.executeBatch(rows);
          await stmt.close();
          await tx.rollback();

          // Verify no rows persisted
          const count = await client.query(`SELECT COUNT(*) AS CNT FROM ${rollbackTable}`);
          const firstRow = count.rows[0];
          const rowCount = firstRow.CNT ?? firstRow.cnt ?? Object.values(firstRow)[0];
          if (rowCount != 0) {
            throw new Error(`Rollback failed — found ${rowCount} rows`);
          }

          return { rowsAffected: rollbackRows, extra: { verified: "0 rows after rollback" } };
        },
      );
      results.push(rollbackResult);
      printResult(rollbackResult);
    } finally {
      await safeDropTable(client, rollbackTable);
    }

    printSummary(results);
  } finally {
    await client.close();
  }
}

main().catch((err) => {
  console.error("Batch benchmark failed:", err);
  process.exit(1);
});
