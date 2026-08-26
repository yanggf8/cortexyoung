import Database from 'better-sqlite3';
import { createHash } from 'node:crypto';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { CortError } from './errors.js';

export const SCHEMA_VERSION = 1;

const SCHEMA_SQL = fs.readFileSync(fileURLToPath(new URL('./schema.sql', import.meta.url)), 'utf8');

export function projectIdFor(realPath) {
  return createHash('sha256').update(realPath).digest('hex');
}

export function dbPathFor(realPath) {
  return path.join(os.homedir(), '.cache', 'cortex-ng', `${projectIdFor(realPath)}.db`);
}

export function openDb(dbPath) {
  if (dbPath !== ':memory:') fs.mkdirSync(path.dirname(dbPath), { recursive: true });
  const db = new Database(dbPath);
  db.pragma('journal_mode = WAL');
  db.pragma('busy_timeout = 5000');
  db.pragma('foreign_keys = ON');
  if (dbPath !== ':memory:') fs.chmodSync(dbPath, 0o600);
  return db;
}

export function ensureSchema(db) {
  db.exec(SCHEMA_SQL);
  const existing = getMeta(db, 'SCHEMA_VERSION');
  if (existing === null) setMeta(db, 'SCHEMA_VERSION', String(SCHEMA_VERSION));
  else if (existing !== String(SCHEMA_VERSION)) {
    throw new CortError('schema_version_mismatch', { found: existing, expected: SCHEMA_VERSION });
  }
}

export function getMeta(db, key) {
  const row = db.prepare('SELECT value FROM _cortex_meta WHERE key = ?').get(key);
  return row ? row.value : null;
}

export function setMeta(db, key, value) {
  db.prepare(`INSERT INTO _cortex_meta (key, value) VALUES (?, ?)
              ON CONFLICT(key) DO UPDATE SET value = excluded.value`).run(key, value);
}

export function withBusyRetry(fn) {
  let lastErr;
  for (let attempt = 0; attempt < 4; attempt += 1) {
    try { return fn(); } catch (err) {
      lastErr = err;
      if (err && err.code === 'SQLITE_BUSY') continue;
      if (err && (err.code === 'SQLITE_FULL' || err.code === 'SQLITE_CORRUPT')) {
        throw new CortError('storage_full', { sqlite_code: err.code });
      }
      throw err;
    }
  }
  throw new CortError('storage_busy', { message: String(lastErr) });
}
