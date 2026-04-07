/**
 * Run All Performance Benchmarks
 *
 * Executes each benchmark suite sequentially and reports overall results.
 *
 * Run:
 *   npm run all
 *
 * Or run individual suites:
 *   npm run crud
 *   npm run batch
 *   npm run pipeline
 *   npm run blob
 *   npm run edge
 *   npm run pool
 */

import { execSync } from "node:child_process";
import { performance } from "node:perf_hooks";
import { resolve, dirname } from "node:path";
import { fileURLToPath } from "node:url";

const __dirname = dirname(fileURLToPath(import.meta.url));
const projectRoot = resolve(__dirname, "..");

const suites = [
  { name: "CRUD Operations", script: "bench-crud" },
  { name: "Large Batch Operations", script: "bench-batch" },
  { name: "Data Pipeline", script: "bench-pipeline" },
  { name: "BLOB / Large Data", script: "bench-blob" },
  { name: "Edge Cases", script: "bench-edge-cases" },
  { name: "Connection Pool Stress", script: "bench-pool" },
];

async function main() {
  console.log("╔══════════════════════════════════════════════════════════════════════╗");
  console.log("║           db2-node Performance & Edge Case Test Suite               ║");
  console.log("╚══════════════════════════════════════════════════════════════════════╝");
  console.log();

  const totalStart = performance.now();
  const suiteResults: { name: string; durationMs: number; passed: boolean; error?: string }[] = [];

  for (const suite of suites) {
    console.log(`\n${"█".repeat(70)}`);
    console.log(`  SUITE: ${suite.name}`);
    console.log(`${"█".repeat(70)}\n`);

    const start = performance.now();
    let passed = true;
    let error: string | undefined;

    try {
      execSync(`npx tsx src/${suite.script}.ts`, {
        cwd: projectRoot,
        stdio: "inherit",
        env: { ...process.env },
        timeout: 600_000, // 10 minute timeout per suite
      });
    } catch (err: any) {
      passed = false;
      error = err.message?.split("\n")[0] || "Unknown error";
    }

    const durationMs = performance.now() - start;
    suiteResults.push({ name: suite.name, durationMs, passed, error });
  }

  const totalMs = performance.now() - totalStart;

  // ── Final report ────────────────────────────────────────────────────
  console.log("\n\n");
  console.log("╔══════════════════════════════════════════════════════════════════════╗");
  console.log("║                        FINAL REPORT                                ║");
  console.log("╚══════════════════════════════════════════════════════════════════════╝");
  console.log();

  const passedCount = suiteResults.filter((r) => r.passed).length;
  const failedCount = suiteResults.filter((r) => !r.passed).length;

  for (const r of suiteResults) {
    const status = r.passed ? "PASS" : "FAIL";
    const icon = r.passed ? "[OK]" : "[!!]";
    const duration = r.durationMs < 1000
      ? `${r.durationMs.toFixed(0)}ms`
      : `${(r.durationMs / 1000).toFixed(1)}s`;

    console.log(`  ${icon} ${status}  ${r.name.padEnd(30)} ${duration}`);
    if (r.error) {
      console.log(`         Error: ${r.error}`);
    }
  }

  console.log();
  console.log(`  Total: ${passedCount} passed, ${failedCount} failed`);
  console.log(`  Duration: ${(totalMs / 1000).toFixed(1)}s`);
  console.log();

  if (failedCount > 0) {
    process.exit(1);
  }
}

main();
