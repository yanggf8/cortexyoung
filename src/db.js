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

export function cacheDir() {
  return process.env.CORT_CACHE_DIR ?? path.join(os.homedir(), '.cache', 'cortex-ng');
}

export function dbPathFor(realPath) {
  return path.join(cacheDir(), `${projectIdFor(realPath)}.db`);
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

export function listProjects() {
  const dir = cacheDir();
  if (!fs.existsSync(dir)) return [];
  const out = [];
  for (const name of fs.readdirSync(dir).sort()) {
    if (!name.endsWith('.db')) continue;
    const dbPath = path.join(dir, name);
    const db = new Database(dbPath, { readonly: true });
    try {
      const row = db.prepare('SELECT project_id, name, path, git_head, last_indexed_at FROM projects').get();
      if (row) out.push({ ...row, db_path: dbPath });
    } catch { /* not a cort db, or schema not created yet */ }
    finally { db.close(); }
  }
  return out;
}

export function deleteProject(realPath) {
  const dbPath = dbPathFor(realPath);
  if (!fs.existsSync(dbPath)) return { deleted: false, db_path: dbPath };
  for (const suffix of ['', '-wal', '-shm']) fs.rmSync(`${dbPath}${suffix}`, { force: true });
  return { deleted: true, db_path: dbPath };
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
