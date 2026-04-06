/**
 * Quick repository demo for db2-node.
 *
 * Run:
 *   npx --yes tsx demo.ts
 *
 * Uses the same DB2_TEST_* environment variables as the integration tests.
 */

import { Client } from "./crates/db2-napi";

const TABLE_NAME = `DEMO_TS_${Date.now()}`;

function createClient() {
  return new Client({
    host: process.env.DB2_TEST_HOST || "localhost",
    port: Number(process.env.DB2_TEST_PORT) || 50000,
    database: process.env.DB2_TEST_DATABASE || "testdb",
    user: process.env.DB2_TEST_USER || "db2inst1",
    password: process.env.DB2_TEST_PASSWORD || "db2wire_test_pw",
  });
}

async function safeDropTable(client: Client, tableName: string) {
  try {
    await client.query(`DROP TABLE ${tableName}`);
  } catch {
    // Ignore cleanup errors so the demo can still exit cleanly.
  }
}

async function main() {
  const client = createClient();
  let connected = false;
  let createdTable = false;

  try {
    await client.connect();
    connected = true;

    const server = await client.serverInfo();
    console.log(`Connected to ${server.productName} ${server.serverRelease}\n`);

    const ping = await client.query(
      "SELECT CURRENT TIMESTAMP AS TS FROM SYSIBM.SYSDUMMY1",
    );
    console.log("Current server timestamp:");
    console.log(ping.rows[0]);
    console.log();

    const employees = await client.query(
      "SELECT id, name, salary FROM employees WHERE id <= ? ORDER BY id",
      [3],
    );
    console.log("First three seeded employees:");
    for (const row of employees.rows) {
      console.log(`  ${JSON.stringify(row)}`);
    }
    console.log();

    const stmt = await client.prepare("VALUES CAST(? AS INTEGER) + 10");
    const prepared = await stmt.execute([32]);
    await stmt.close();
    console.log("Prepared statement result:");
    console.log(prepared.rows[0]);
    console.log();

    await client.query(
      `CREATE TABLE ${TABLE_NAME} (
        id INTEGER NOT NULL,
        name VARCHAR(64) NOT NULL,
        score DECIMAL(5,2)
      )`,
    );
    createdTable = true;

    const tx = await client.beginTransaction();
    const insert = await tx.prepare(
      `INSERT INTO ${TABLE_NAME} (id, name, score) VALUES (?, ?, ?)`,
    );

    await insert.executeBatch([
      [1, "Ada", 98.5],
      [2, "Linus", 96.0],
      [3, "Grace", 99.25],
    ]);
    await insert.close();
    await tx.commit();

    const inserted = await client.query(
      `SELECT id, name, score FROM ${TABLE_NAME} ORDER BY id`,
    );
    console.log("Rows inserted inside a transaction:");
    for (const row of inserted.rows) {
      console.log(`  ${JSON.stringify(row)}`);
    }
    console.log();

    console.log("Demo completed successfully.");
  } finally {
    if (connected && createdTable) {
      await safeDropTable(client, TABLE_NAME);
    }

    if (connected) {
      await client.close();
    }
  }
}

main().catch((error) => {
  console.error("Demo failed:", error);
  process.exit(1);
});
