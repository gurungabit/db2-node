/**
 * Bulk insert/read benchmark for db2-node.
 *
 * Run:
 *   DEMO_TOTAL_ROWS=100000 npx --yes tsx demo-million.ts
 *
 * Uses the same DB2_TEST_* environment variables as the integration tests.
 */

import { Client } from "./crates/db2-napi";

const TOTAL_ROWS = Number(process.env.DEMO_TOTAL_ROWS || 1_000_000);
const BATCH_SIZE = Number(process.env.DEMO_BATCH_SIZE || 5_000);
const COMMIT_EVERY = Number(process.env.DEMO_COMMIT_EVERY || 50_000);
const TABLE_NAME = sanitizeIdentifier(
  process.env.DEMO_TABLE_NAME || `demo_bulk_${Date.now()}`,
);

function sanitizeIdentifier(value: string): string {
  return value.replace(/[^A-Za-z0-9_]/g, "_").toUpperCase();
}

function createClient() {
  return new Client({
    host: process.env.DB2_TEST_HOST || "localhost",
    port: Number(process.env.DB2_TEST_PORT) || 50000,
    database: process.env.DB2_TEST_DATABASE || "testdb",
    user: process.env.DB2_TEST_USER || "db2inst1",
    password: process.env.DB2_TEST_PASSWORD || "db2wire_test_pw",
  });
}

function elapsedSeconds(startMs: number): number {
  return (performance.now() - startMs) / 1000;
}

function formatRate(rows: number, seconds: number): string {
  if (seconds <= 0) {
    return "0";
  }

  return Math.round(rows / seconds).toLocaleString();
}

function isLogFullError(error: unknown): boolean {
  const message =
    error &&
    typeof error === "object" &&
    "message" in error &&
    typeof error.message === "string"
      ? error.message
      : String(error);

  return message.includes("SQLSTATE=57011") || message.includes("SQLCODE=-964");
}

async function safeDropTable(client: Client, tableName: string) {
  try {
    await client.query(`DROP TABLE ${tableName}`);
  } catch {
    // Ignore cleanup failures during demo teardown.
  }
}

async function main() {
  const client = createClient();
  let connected = false;
  let tableCreated = false;

  try {
    await client.connect();
    connected = true;

    const server = await client.serverInfo();
    console.log(`Connected to ${server.productName} ${server.serverRelease}`);
    console.log(
      `Loading ${TOTAL_ROWS.toLocaleString()} rows into ${TABLE_NAME} ` +
        `(batch=${BATCH_SIZE.toLocaleString()}, commitEvery=${COMMIT_EVERY.toLocaleString()})\n`,
    );

    await safeDropTable(client, TABLE_NAME);
    await client.query(`
      CREATE TABLE ${TABLE_NAME} (
        id      INTEGER NOT NULL,
        val     INTEGER NOT NULL,
        label   VARCHAR(60) NOT NULL,
        score   DECIMAL(10,2),
        created TIMESTAMP DEFAULT CURRENT_TIMESTAMP
      )
    `);
    tableCreated = true;

    const insertSql = `INSERT INTO ${TABLE_NAME} (id, val, label, score) VALUES (?, ?, ?, ?)`;
    const writeStart = performance.now();
    let inserted = 0;
    let rowsSinceCommit = 0;
    let tx = await client.beginTransaction();
    let stmt = await tx.prepare(insertSql);

    try {
      while (inserted < TOTAL_ROWS) {
        const nextBatchSize = Math.min(BATCH_SIZE, TOTAL_ROWS - inserted);
        const paramRows: Array<[number, number, string, number]> = [];

        for (let offset = 0; offset < nextBatchSize; offset += 1) {
          const id = inserted + offset + 1;
          paramRows.push([
            id,
            (id * 7) % 10_000,
            `Row number ${id}`,
            (id % 1000) / 100,
          ]);
        }

        await stmt.executeBatch(paramRows);
        inserted += nextBatchSize;
        rowsSinceCommit += nextBatchSize;

        if (rowsSinceCommit >= COMMIT_EVERY || inserted >= TOTAL_ROWS) {
          await stmt.close();
          await tx.commit();

          const seconds = elapsedSeconds(writeStart);
          process.stdout.write(
            `\r  inserted ${inserted.toLocaleString()} / ${TOTAL_ROWS.toLocaleString()} rows ` +
              `(${seconds.toFixed(1)}s, ${formatRate(inserted, seconds)} rows/sec)`,
          );

          rowsSinceCommit = 0;

          if (inserted < TOTAL_ROWS) {
            tx = await client.beginTransaction();
            stmt = await tx.prepare(insertSql);
          }
        }
      }
    } catch (error) {
      try {
        await stmt.close();
      } catch {
        // Ignore statement-close errors while unwinding.
      }

      try {
        await tx.rollback();
      } catch {
        // Ignore rollback errors while unwinding.
      }

      if (isLogFullError(error)) {
        console.error(
          "\nDB2 transaction log filled during the bulk load. Try a smaller commit interval:",
        );
        console.error(
          "  DEMO_COMMIT_EVERY=10000 DEMO_BATCH_SIZE=2000 npx --yes tsx demo-million.ts",
        );
        console.error("For the local Docker DB2 instance, run: ./tools/db2.sh tune");
      }

      throw error;
    }

    const writeSeconds = elapsedSeconds(writeStart);
    console.log(
      `\n\nInsert complete in ${writeSeconds.toFixed(2)}s ` +
        `(${formatRate(TOTAL_ROWS, writeSeconds)} rows/sec)\n`,
    );

    const count = await client.query(`SELECT COUNT(*) AS TOTAL FROM ${TABLE_NAME}`);
    console.log(`Verified row count: ${count.rows[0].TOTAL}`);

    const middleId = Math.max(1, Math.floor(TOTAL_ROWS / 2));
    const sample = await client.query(
      `SELECT id, val, label, score FROM ${TABLE_NAME} WHERE id IN (?, ?, ?) ORDER BY id`,
      [1, middleId, TOTAL_ROWS],
    );

    console.log("Sample rows:");
    for (const row of sample.rows) {
      console.log(`  ${JSON.stringify(row)}`);
    }

    console.log(`\nReading all ${TOTAL_ROWS.toLocaleString()} rows back...`);
    const readStart = performance.now();
    const allRows = await client.query(
      `SELECT id, val, label, score FROM ${TABLE_NAME} ORDER BY id`,
    );
    const readSeconds = elapsedSeconds(readStart);

    console.log(
      `Read ${allRows.rows.length.toLocaleString()} rows in ${readSeconds.toFixed(2)}s ` +
        `(${formatRate(allRows.rows.length, readSeconds)} rows/sec)`,
    );

    if (allRows.rows.length !== TOTAL_ROWS) {
      throw new Error(
        `Row count mismatch: expected ${TOTAL_ROWS}, got ${allRows.rows.length}`,
      );
    }
  } finally {
    if (connected && tableCreated) {
      console.log(`\nCleaning up ${TABLE_NAME}...`);
      await safeDropTable(client, TABLE_NAME);
    }

    if (connected) {
      await client.close();
    }
  }
}

main().catch((error) => {
  console.error("Benchmark failed:", error);
  process.exit(1);
});
