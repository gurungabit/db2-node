/**
 * Shared test helpers for DB2 Wire Driver test suite.
 *
 * Mirrors the patterns from ibm_db/test but adapted
 * for our napi-rs based driver API.
 */

// Re-export the driver classes under both raw and friendly names
export {
  Client,
  Pool,
  PreparedStatement,
  Transaction,
} from '../../crates/db2-napi';

export type {
  ConnectionConfig,
  PoolConfig,
  QueryResult,
  ColumnInfo,
  ServerInfo,
} from '../../crates/db2-napi';

import { Client } from '../../crates/db2-napi';
import type { ConnectionConfig } from '../../crates/db2-napi';

/** Default connection config from environment or Docker defaults. */
export function getConfig(): ConnectionConfig {
  return {
    host: process.env.DB2_TEST_HOST || 'localhost',
    port: Number(process.env.DB2_TEST_PORT) || 50000,
    database: process.env.DB2_TEST_DATABASE || 'testdb',
    user: process.env.DB2_TEST_USER || 'db2inst1',
    password: process.env.DB2_TEST_PASSWORD || 'db2wire_test_pw',
  };
}

/** Create a connected client ready for testing. */
export async function connect(): Promise<InstanceType<typeof Client>> {
  const client = new Client(getConfig());
  await client.connect();
  return client;
}

/** Generate a unique temporary table name to avoid collisions. */
export function tempTable(prefix = 'tmp'): string {
  return `${prefix}_${Date.now() % 1_000_000}`;
}

/** Run SQL ignoring errors (useful for DROP IF EXISTS). */
export async function execIgnore(
  client: InstanceType<typeof Client>,
  sql: string,
): Promise<void> {
  try {
    await client.query(sql);
  } catch {
    // ignore
  }
}
