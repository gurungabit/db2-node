/**
 * Connection Pool Stress Test
 *
 * Tests pool behavior under pressure:
 *   - Pool warmup / connection ramp
 *   - Max connection saturation
 *   - Acquire/release churn
 *   - Concurrent heavy workloads
 *   - Pool stats accuracy
 *   - Mixed read/write through pool
 *   - Connection reuse patterns
 *
 * Run:
 *   npm run pool
 */

import { Pool, Client } from "@gurungabit/db2-node";
import {
  createPool,
  poolConfig,
  uniqueTable,
  safeDropTablePool,
  section,
  runBench,
  printResult,
  printSummary,
  randomString,
  randomInt,
  formatMs,
  elapsed,
  type BenchResult,
} from "./helpers.js";

async function main() {
  const results: BenchResult[] = [];

  // ── Pool warmup ─────────────────────────────────────────────────────
  section("Pool warmup (min connections)");

  const pool = createPool({ minConnections: 5, maxConnections: 10 });

  const warmupResult = await runBench("Pool creation + first query", async () => {
    const res = await pool.query("VALUES 1");
    const stats = {
      idle: await pool.idleCount(),
      active: await pool.activeCount(),
      total: await pool.totalCount(),
      max: pool.maxConnections(),
    };
    return { rowsAffected: 1, extra: stats };
  });
  results.push(warmupResult);
  printResult(warmupResult);

  // ── Sequential acquire/release ────────────────────────────────────
  section("Acquire/release cycle (sequential)");

  const acquireReleaseResult = await runBench("200 acquire → query → release", async () => {
    for (let i = 0; i < 200; i++) {
      const client = await pool.acquire();
      await client.query("VALUES 1");
      await pool.release(client);
    }
    return { rowsAffected: 200 };
  });
  results.push(acquireReleaseResult);
  printResult(acquireReleaseResult);

  // ── Concurrent pool saturation ────────────────────────────────────
  section("Pool saturation — 10 concurrent (max=10)");

  const saturationResult = await runBench("50 rounds x 10 concurrent queries", async () => {
    let total = 0;
    for (let round = 0; round < 50; round++) {
      const promises = Array.from({ length: 10 }, () =>
        pool.query("VALUES (CURRENT_TIMESTAMP)"),
      );
      await Promise.all(promises);
      total += 10;
    }
    return {
      rowsAffected: total,
      extra: {
        idle: await pool.idleCount(),
        active: await pool.activeCount(),
      },
    };
  });
  results.push(saturationResult);
  printResult(saturationResult);

  // ── Beyond max connections (queuing) ──────────────────────────────
  section("Beyond max connections — 20 concurrent (max=10, expect queuing)");

  const queuingResult = await runBench("20 concurrent queries on max=10 pool", async () => {
    const promises = Array.from({ length: 20 }, (_, i) =>
      pool.query(`VALUES (${i + 1})`),
    );
    const allResults = await Promise.all(promises);
    return { rowsAffected: allResults.length };
  });
  results.push(queuingResult);
  printResult(queuingResult);

  // ── Heavy concurrent read/write through pool ──────────────────────
  section("Mixed read/write workload through pool");

  const mixedTable = uniqueTable("POOL_MIX");
  try {
    await pool.query(`
      CREATE TABLE ${mixedTable} (
        id   INTEGER NOT NULL,
        data VARCHAR(200)
      )
    `);

    // Seed
    const seedClient = await pool.acquire();
    const tx = await seedClient.beginTransaction();
    const stmt = await tx.prepare(`INSERT INTO ${mixedTable} (id, data) VALUES (?, ?)`);
    const seedRows: any[][] = [];
    for (let i = 0; i < 5_000; i++) {
      seedRows.push([i + 1, randomString(100)]);
    }
    await stmt.executeBatch(seedRows);
    await stmt.close();
    await tx.commit();
    await pool.release(seedClient);

    const mixedResult = await runBench("100 concurrent mixed read/write ops", async () => {
      let reads = 0;
      let writes = 0;

      // Run in smaller batches to avoid deadlocks from concurrent UPDATEs
      for (let batch = 0; batch < 10; batch++) {
        const promises = Array.from({ length: 10 }, async (_, i) => {
          const idx = batch * 10 + i;
          if (idx % 4 === 0) {
            // Write — update a unique row per task to avoid deadlock
            await pool.query(
              `UPDATE ${mixedTable} SET data = ? WHERE id = ?`,
              [randomString(80), idx + 1],
            );
            writes++;
          } else {
            // Read
            const start = randomInt(1, 4900);
            const res = await pool.query(
              `SELECT id, data FROM ${mixedTable} WHERE id BETWEEN ? AND ?`,
              [start, start + 100],
            );
            reads += res.rows.length;
          }
        });
        await Promise.all(promises);
      }

      return { rowsAffected: reads + writes, extra: { reads, writes } };
    });
    results.push(mixedResult);
    printResult(mixedResult);
  } finally {
    await safeDropTablePool(pool, mixedTable);
  }

  // ── Acquire with transaction ──────────────────────────────────────
  section("Pool acquire → transaction → release");

  const txTable = uniqueTable("POOL_TX");
  try {
    await pool.query(`
      CREATE TABLE ${txTable} (id INTEGER NOT NULL, val INTEGER)
    `);

    const txPoolResult = await runBench("20 acquire → tx → commit → release", async () => {
      for (let i = 0; i < 20; i++) {
        const client = await pool.acquire();
        const tx = await client.beginTransaction();
        await tx.query(`INSERT INTO ${txTable} (id, val) VALUES (?, ?)`, [i + 1, i * 10]);
        await tx.commit();
        await pool.release(client);
      }

      const count = await pool.query(`SELECT COUNT(*) AS CNT FROM ${txTable}`);
      const firstRow = count.rows[0];
      return { rowsAffected: 20, extra: { rowsInTable: firstRow.CNT ?? firstRow.cnt ?? Object.values(firstRow)[0] } };
    });
    results.push(txPoolResult);
    printResult(txPoolResult);
  } finally {
    await safeDropTablePool(pool, txTable);
  }

  // ── Pool stats tracking ───────────────────────────────────────────
  section("Pool stats after workload");

  const statsResult = await runBench("Collect pool stats", async () => {
    const stats = {
      idle: await pool.idleCount(),
      active: await pool.activeCount(),
      total: await pool.totalCount(),
      maxConnections: pool.maxConnections(),
    };
    return { rowsAffected: 1, extra: stats };
  });
  results.push(statsResult);
  printResult(statsResult);

  // ── Multiple pools ────────────────────────────────────────────────
  section("Multiple pools (3 pools x 5 max connections)");

  const pools = [
    createPool({ maxConnections: 5, minConnections: 1 }),
    createPool({ maxConnections: 5, minConnections: 1 }),
    createPool({ maxConnections: 5, minConnections: 1 }),
  ];

  const multiPoolResult = await runBench("30 queries across 3 pools", async () => {
    const promises: Promise<any>[] = [];
    for (let i = 0; i < 30; i++) {
      const p = pools[i % 3];
      promises.push(p.query("VALUES (CURRENT_TIMESTAMP)"));
    }
    await Promise.all(promises);
    return { rowsAffected: 30, extra: { poolCount: 3 } };
  });
  results.push(multiPoolResult);
  printResult(multiPoolResult);

  for (const p of pools) await p.close();

  // ── Rapid pool create / destroy ───────────────────────────────────
  section("Rapid pool create/destroy");

  const poolChurnResult = await runBench("20 pool create → query → close cycles", async () => {
    for (let i = 0; i < 20; i++) {
      const p = createPool({ maxConnections: 3, minConnections: 1 });
      await p.query("VALUES 1");
      await p.close();
    }
    return { rowsAffected: 20 };
  });
  results.push(poolChurnResult);
  printResult(poolChurnResult);

  // Cleanup main pool
  await pool.close();

  printSummary(results);
}

main().catch((err) => {
  console.error("Pool benchmark failed:", err);
  process.exit(1);
});
