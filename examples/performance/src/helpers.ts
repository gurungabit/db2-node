/**
 * Shared helpers for the performance benchmark suite.
 */

import { Client, Pool, type ConnectionConfig, type PoolConfig } from "@gurungabit/db2-node";

// ---------------------------------------------------------------------------
// Connection config
// ---------------------------------------------------------------------------

export function connectionConfig(): ConnectionConfig {
  return {
    host: process.env.DB2_TEST_HOST || "localhost",
    port: Number(process.env.DB2_TEST_PORT) || 50000,
    database: process.env.DB2_TEST_DATABASE || "testdb",
    user: process.env.DB2_TEST_USER || "db2inst1",
    password: process.env.DB2_TEST_PASSWORD || "db2wire_test_pw",
  };
}

export function poolConfig(overrides: Partial<PoolConfig> = {}): PoolConfig {
  return {
    ...connectionConfig(),
    minConnections: 2,
    maxConnections: 10,
    idleTimeout: 30_000,
    maxLifetime: 120_000,
    ...overrides,
  };
}

export function createClient(): Client {
  return new Client(connectionConfig());
}

export function createPool(overrides: Partial<PoolConfig> = {}): Pool {
  return new Pool(poolConfig(overrides));
}

// ---------------------------------------------------------------------------
// Timing / reporting
// ---------------------------------------------------------------------------

export interface BenchResult {
  name: string;
  durationMs: number;
  rowsAffected?: number;
  opsPerSec?: number;
  extra?: Record<string, unknown>;
}

export function elapsed(startMs: number): number {
  return performance.now() - startMs;
}

export function formatMs(ms: number): string {
  if (ms < 1000) return `${ms.toFixed(1)}ms`;
  return `${(ms / 1000).toFixed(2)}s`;
}

export function formatRate(count: number, ms: number): string {
  if (ms <= 0) return "0";
  return Math.round((count / ms) * 1000).toLocaleString();
}

export function printResult(r: BenchResult) {
  const parts = [`  ${r.name}: ${formatMs(r.durationMs)}`];
  if (r.rowsAffected != null) {
    parts.push(`  rows=${r.rowsAffected.toLocaleString()}`);
  }
  if (r.opsPerSec != null) {
    parts.push(`  ${r.opsPerSec.toLocaleString()} ops/sec`);
  }
  if (r.extra) {
    for (const [k, v] of Object.entries(r.extra)) {
      parts.push(`  ${k}=${v}`);
    }
  }
  console.log(parts.join(""));
}

export function printSummary(results: BenchResult[]) {
  console.log("\n" + "=".repeat(70));
  console.log("SUMMARY");
  console.log("=".repeat(70));
  for (const r of results) {
    printResult(r);
  }
  console.log("=".repeat(70));
}

// ---------------------------------------------------------------------------
// Table helpers
// ---------------------------------------------------------------------------

const TS = Date.now();
let counter = 0;

export function uniqueTable(prefix: string): string {
  counter++;
  return `PERF_${prefix}_${TS}_${counter}`.toUpperCase();
}

export async function safeDropTable(client: Client, table: string) {
  try {
    await client.query(`DROP TABLE ${table}`);
  } catch {
    // Ignore — table may not exist.
  }
}

export async function safeDropTablePool(pool: Pool, table: string) {
  try {
    await pool.query(`DROP TABLE ${table}`);
  } catch {
    // Ignore — table may not exist.
  }
}

// ---------------------------------------------------------------------------
// Data generation
// ---------------------------------------------------------------------------

export function randomString(length: number): string {
  const chars = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789";
  let result = "";
  for (let i = 0; i < length; i++) {
    result += chars.charAt(Math.floor(Math.random() * chars.length));
  }
  return result;
}

export function randomInt(min: number, max: number): number {
  return Math.floor(Math.random() * (max - min + 1)) + min;
}

export function generateBlob(sizeBytes: number): Buffer {
  const buf = Buffer.alloc(sizeBytes);
  for (let i = 0; i < sizeBytes; i++) {
    buf[i] = Math.floor(Math.random() * 256);
  }
  return buf;
}

// ---------------------------------------------------------------------------
// Benchmark runner
// ---------------------------------------------------------------------------

export async function runBench(
  name: string,
  fn: () => Promise<{ rowsAffected?: number; extra?: Record<string, unknown> }>,
): Promise<BenchResult> {
  const start = performance.now();
  const result = await fn();
  const durationMs = elapsed(start);
  const opsPerSec =
    result.rowsAffected != null
      ? Math.round((result.rowsAffected / durationMs) * 1000)
      : undefined;

  return {
    name,
    durationMs,
    rowsAffected: result.rowsAffected,
    opsPerSec,
    extra: result.extra,
  };
}

// ---------------------------------------------------------------------------
// Section header
// ---------------------------------------------------------------------------

export function section(title: string) {
  console.log(`\n${"─".repeat(70)}`);
  console.log(`  ${title}`);
  console.log(`${"─".repeat(70)}`);
}
