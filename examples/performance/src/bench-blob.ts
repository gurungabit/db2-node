/**
 * BLOB / Large Data Benchmark
 *
 * Tests handling of large text fields, wide rows with many columns,
 * mixed data types, and NULL-heavy sparse data.
 *
 * Run:
 *   npm run blob
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
  type BenchResult,
} from "./helpers.js";

async function main() {
  const client = createClient();
  const results: BenchResult[] = [];

  try {
    await client.connect();
    const info = await client.serverInfo();
    console.log(`Connected to ${info.productName} ${info.serverRelease}`);
    console.log("BLOB / Large Data benchmark\n");

    // ── BLOB via hex literals ───────────────────────────────────────────
    section("BLOB — insert and read binary data (hex literals)");

    const blobTable = uniqueTable("BLOB");
    try {
      await safeDropTable(client, blobTable);
      await client.query(`
        CREATE TABLE ${blobTable} (
          id   INTEGER NOT NULL,
          name VARCHAR(50) NOT NULL,
          data BLOB(1M)
        )
      `);

      // Insert BLOBs using hex literals (driver doesn't support Buffer params)
      // Note: hex literal in SQL has 2 chars per byte, plus overhead.
      // DB2 has a limit on string constant length (~32KB), so max ~15KB via hex.
      const blobSizes = [
        { label: "1KB", bytes: 1_024 },
        { label: "4KB", bytes: 4_096 },
        { label: "10KB", bytes: 10_240 },
      ];

      for (let i = 0; i < blobSizes.length; i++) {
        const { label, bytes } = blobSizes[i];

        const insertR = await runBench(`INSERT BLOB ${label}`, async () => {
          // Generate random hex string
          const hexChars = "0123456789ABCDEF";
          let hex = "";
          for (let j = 0; j < bytes * 2; j++) {
            hex += hexChars[Math.floor(Math.random() * 16)];
          }
          await client.query(
            `INSERT INTO ${blobTable} (id, name, data) VALUES (?, ?, BLOB(X'${hex}'))`,
            [i + 1, `blob_${label}`],
          );
          return { rowsAffected: 1, extra: { sizeBytes: bytes } };
        });
        results.push(insertR);
        printResult(insertR);
      }

      // Read BLOBs back
      const readBlobResult = await runBench("READ all BLOBs", async () => {
        const res = await client.query(`SELECT id, name, data FROM ${blobTable} ORDER BY id`);
        let totalBytes = 0;
        for (const row of res.rows) {
          if (row.DATA && row.DATA.length) {
            totalBytes += row.DATA.length;
          } else if (row.data && row.data.length) {
            totalBytes += row.data.length;
          }
        }
        return { rowsAffected: res.rows.length, extra: { totalBytes } };
      });
      results.push(readBlobResult);
      printResult(readBlobResult);

      // Batch insert many small BLOBs via hex
      const smallBlobCount = 20;
      const batchBlobResult = await runBench(
        `Batch INSERT ${smallBlobCount} x 1KB BLOBs (hex)`,
        async () => {
          const hexChars = "0123456789ABCDEF";
          for (let i = 0; i < smallBlobCount; i++) {
            let hex = "";
            for (let j = 0; j < 2048; j++) { // 1KB = 1024 bytes = 2048 hex chars
              hex += hexChars[Math.floor(Math.random() * 16)];
            }
            await client.query(
              `INSERT INTO ${blobTable} (id, name, data) VALUES (?, ?, BLOB(X'${hex}'))`,
              [100 + i, `small_${i}`],
            );
          }
          return { rowsAffected: smallBlobCount };
        },
      );
      results.push(batchBlobResult);
      printResult(batchBlobResult);
    } finally {
      await safeDropTable(client, blobTable);
    }

    // ── Large VARCHAR / CLOB ────────────────────────────────────────────
    section("Large text fields (VARCHAR / CLOB)");

    const textTable = uniqueTable("TEXT");
    try {
      await safeDropTable(client, textTable);
      await client.query(`
        CREATE TABLE ${textTable} (
          id      INTEGER NOT NULL,
          title   VARCHAR(200),
          body    VARCHAR(32000)
        )
      `);

      // Insert large text
      const textSizes = [
        { label: "1KB text", chars: 1_000 },
        { label: "10KB text", chars: 10_000 },
        { label: "30KB text", chars: 30_000 },
      ];

      for (let i = 0; i < textSizes.length; i++) {
        const { label, chars } = textSizes[i];

        const r = await runBench(`INSERT ${label}`, async () => {
          const text = randomString(chars);
          await client.query(
            `INSERT INTO ${textTable} (id, title, body) VALUES (?, ?, ?)`,
            [i + 1, `doc_${label}`, text],
          );
          return { rowsAffected: 1, extra: { chars } };
        });
        results.push(r);
        printResult(r);
      }

      // Read back large text
      const readTextResult = await runBench("READ all large text", async () => {
        const res = await client.query(`SELECT id, title, body FROM ${textTable} ORDER BY id`);
        let totalChars = 0;
        for (const row of res.rows) {
          const body = row.BODY ?? row.body;
          if (body) totalChars += String(body).length;
        }
        return { rowsAffected: res.rows.length, extra: { totalChars } };
      });
      results.push(readTextResult);
      printResult(readTextResult);
    } finally {
      await safeDropTable(client, textTable);
    }

    // ── Wide rows (many columns) ────────────────────────────────────────
    section("Wide rows — 50 columns");

    const wideTable = uniqueTable("WIDE");
    const numCols = 50;
    const wideRowCount = 5_000;

    try {
      await safeDropTable(client, wideTable);

      const colDefs = ["id INTEGER NOT NULL"];
      for (let i = 1; i <= numCols; i++) {
        if (i % 3 === 0) colDefs.push(`col_${i} DECIMAL(10,2)`);
        else if (i % 3 === 1) colDefs.push(`col_${i} VARCHAR(50)`);
        else colDefs.push(`col_${i} INTEGER`);
      }
      await client.query(`CREATE TABLE ${wideTable} (${colDefs.join(", ")})`);

      // Insert wide rows
      const placeholders = ["?", ...Array(numCols).fill("?")].join(", ");
      const wideInsertResult = await runBench(
        `INSERT ${wideRowCount.toLocaleString()} wide rows`,
        async () => {
          let inserted = 0;
          while (inserted < wideRowCount) {
            const batchEnd = Math.min(inserted + 1000, wideRowCount);
            const tx = await client.beginTransaction();
            const stmt = await tx.prepare(
              `INSERT INTO ${wideTable} VALUES (${placeholders})`,
            );

            const rows: any[][] = [];
            for (let i = inserted; i < batchEnd; i++) {
              const row: any[] = [i + 1];
              for (let c = 1; c <= numCols; c++) {
                if (c % 3 === 0) row.push(randomInt(1, 99999) / 100);
                else if (c % 3 === 1) row.push(randomString(randomInt(5, 40)));
                else row.push(randomInt(1, 1_000_000));
              }
              rows.push(row);
            }

            await stmt.executeBatch(rows);
            await stmt.close();
            await tx.commit();
            inserted = batchEnd;
          }
          return { rowsAffected: wideRowCount };
        },
      );
      results.push(wideInsertResult);
      printResult(wideInsertResult);

      // Read all wide rows
      const wideReadResult = await runBench("READ all wide rows", async () => {
        const res = await client.query(`SELECT * FROM ${wideTable}`);
        return {
          rowsAffected: res.rows.length,
          extra: { columns: res.columns.length },
        };
      });
      results.push(wideReadResult);
      printResult(wideReadResult);
    } finally {
      await safeDropTable(client, wideTable);
    }

    // ── Mixed data types row ────────────────────────────────────────────
    section("Mixed data types (all supported types)");

    const mixedTable = uniqueTable("MIXED");
    const mixedRows = 2_000;

    try {
      await safeDropTable(client, mixedTable);
      await client.query(`
        CREATE TABLE ${mixedTable} (
          id            INTEGER NOT NULL,
          tiny_val      SMALLINT,
          big_val       BIGINT,
          float_val     REAL,
          double_val    DOUBLE,
          decimal_val   DECIMAL(18,6),
          char_val      CHAR(10),
          varchar_val   VARCHAR(200),
          date_val      DATE,
          time_val      TIME,
          ts_val        TIMESTAMP,
          bool_val      SMALLINT
        )
      `);

      const mixedInsertResult = await runBench(
        `INSERT ${mixedRows.toLocaleString()} mixed-type rows`,
        async () => {
          const tx = await client.beginTransaction();
          const stmt = await tx.prepare(
            `INSERT INTO ${mixedTable} VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)`,
          );

          const rows: any[][] = [];
          for (let i = 0; i < mixedRows; i++) {
            rows.push([
              i + 1,
              randomInt(-32768, 32767),              // SMALLINT
              randomInt(1, 999999999) * 1000,         // BIGINT (use Number, not BigInt)
              Math.random() * 1000,                   // REAL
              Math.random() * 1e12,                   // DOUBLE
              randomInt(1, 999999999) / 1e6,          // DECIMAL
              randomString(10),                       // CHAR(10)
              randomString(randomInt(10, 150)),        // VARCHAR
              "2024-06-15",                           // DATE
              "14:30:00",                             // TIME
              "2024-06-15 14:30:00.123456",           // TIMESTAMP
              randomInt(0, 1),                        // BOOLEAN-ish
            ]);
          }

          await stmt.executeBatch(rows);
          await stmt.close();
          await tx.commit();
          return { rowsAffected: mixedRows };
        },
      );
      results.push(mixedInsertResult);
      printResult(mixedInsertResult);

      const mixedReadResult = await runBench("READ mixed-type rows", async () => {
        const res = await client.query(`SELECT * FROM ${mixedTable}`);
        return { rowsAffected: res.rows.length, extra: { columns: res.columns.length } };
      });
      results.push(mixedReadResult);
      printResult(mixedReadResult);
    } finally {
      await safeDropTable(client, mixedTable);
    }

    // ── NULL-heavy data ─────────────────────────────────────────────────
    section("NULL-heavy data (sparse rows)");

    const nullTable = uniqueTable("NULLS");
    const nullRows = 5_000;

    try {
      await safeDropTable(client, nullTable);
      await client.query(`
        CREATE TABLE ${nullTable} (
          id   INTEGER NOT NULL,
          a    VARCHAR(100),
          b    INTEGER,
          c    DECIMAL(10,2),
          d    VARCHAR(200),
          e    TIMESTAMP
        )
      `);

      const nullInsertResult = await runBench(
        `INSERT ${nullRows} sparse rows (80% NULLs)`,
        async () => {
          const tx = await client.beginTransaction();
          const stmt = await tx.prepare(
            `INSERT INTO ${nullTable} VALUES (?, ?, ?, ?, ?, ?)`,
          );
          const rows: any[][] = [];
          for (let i = 0; i < nullRows; i++) {
            rows.push([
              i + 1,
              Math.random() < 0.8 ? null : randomString(50),
              Math.random() < 0.8 ? null : randomInt(1, 9999),
              Math.random() < 0.8 ? null : randomInt(1, 99999) / 100,
              Math.random() < 0.8 ? null : randomString(100),
              Math.random() < 0.8 ? null : "2024-01-01 00:00:00",
            ]);
          }
          await stmt.executeBatch(rows);
          await stmt.close();
          await tx.commit();
          return { rowsAffected: nullRows };
        },
      );
      results.push(nullInsertResult);
      printResult(nullInsertResult);

      const nullReadResult = await runBench("READ sparse rows", async () => {
        const res = await client.query(`SELECT * FROM ${nullTable}`);
        let nullCount = 0;
        for (const row of res.rows) {
          for (const key of Object.keys(row)) {
            if (row[key] == null) nullCount++;
          }
        }
        return { rowsAffected: res.rows.length, extra: { totalNulls: nullCount } };
      });
      results.push(nullReadResult);
      printResult(nullReadResult);
    } finally {
      await safeDropTable(client, nullTable);
    }

    printSummary(results);
  } finally {
    await client.close();
  }
}

main().catch((err) => {
  console.error("BLOB benchmark failed:", err);
  process.exit(1);
});
