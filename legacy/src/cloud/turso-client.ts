import { createClient, type Client } from '@libsql/client';
import type { ProjectContextInfo } from '../types/patterns.js';

export interface SnapshotRow {
  projectId: string;
  machineId: string;
  context: ProjectContextInfo;
  scannedAt: string;
}

const MIGRATION_SQL = [
  `CREATE TABLE IF NOT EXISTS projects (
    id          TEXT PRIMARY KEY,
    display_name TEXT NOT NULL,
    created_at  TEXT NOT NULL,
    updated_at  TEXT NOT NULL
  )`,
  `CREATE TABLE IF NOT EXISTS pattern_snapshots (
    id          INTEGER PRIMARY KEY,
    project_id  TEXT NOT NULL REFERENCES projects(id),
    machine_id  TEXT NOT NULL,
    context     TEXT NOT NULL,
    scanned_at  TEXT NOT NULL,
    created_at  TEXT NOT NULL,
    UNIQUE(project_id, machine_id)
  )`,
  `CREATE INDEX IF NOT EXISTS idx_snapshots_project_scanned
    ON pattern_snapshots(project_id, scanned_at DESC)`,
];

export class TursoClient {
  private client: Client;

  constructor(url: string, authToken: string) {
    this.client = createClient({ url, authToken });
  }

  async migrate(): Promise<void> {
    for (const sql of MIGRATION_SQL) {
      await this.client.execute(sql);
    }
  }

  async upsertProject(id: string, displayName: string): Promise<void> {
    const now = new Date().toISOString();
    await this.client.execute({
      sql: `INSERT INTO projects (id, display_name, created_at, updated_at)
            VALUES (?, ?, ?, ?)
            ON CONFLICT(id) DO UPDATE SET display_name = excluded.display_name, updated_at = excluded.updated_at`,
      args: [id, displayName, now, now],
    });
  }

  async pushSnapshot(projectId: string, machineId: string, context: ProjectContextInfo): Promise<void> {
    const now = new Date().toISOString();
    await this.client.execute({
      sql: `INSERT INTO pattern_snapshots (project_id, machine_id, context, scanned_at, created_at)
            VALUES (?, ?, ?, ?, ?)
            ON CONFLICT(project_id, machine_id)
            DO UPDATE SET context = excluded.context, scanned_at = excluded.scanned_at`,
      args: [projectId, machineId, JSON.stringify(context), context.lastUpdated, now],
    });
  }

  async getLatestSnapshot(projectId: string): Promise<SnapshotRow | null> {
    const result = await this.client.execute({
      sql: `SELECT project_id, machine_id, context, scanned_at
            FROM pattern_snapshots
            WHERE project_id = ?
            ORDER BY scanned_at DESC
            LIMIT 1`,
      args: [projectId],
    });

    if (result.rows.length === 0) return null;

    const row = result.rows[0];
    return {
      projectId: row['project_id'] as string,
      machineId: row['machine_id'] as string,
      context: JSON.parse(row['context'] as string) as ProjectContextInfo,
      scannedAt: row['scanned_at'] as string,
    };
  }

  async getAllSnapshots(projectId: string): Promise<SnapshotRow[]> {
    const result = await this.client.execute({
      sql: `SELECT project_id, machine_id, context, scanned_at
            FROM pattern_snapshots
            WHERE project_id = ?
            ORDER BY scanned_at DESC`,
      args: [projectId],
    });

    return result.rows.map((row) => ({
      projectId: row['project_id'] as string,
      machineId: row['machine_id'] as string,
      context: JSON.parse(row['context'] as string) as ProjectContextInfo,
      scannedAt: row['scanned_at'] as string,
    }));
  }

  close(): void {
    this.client.close();
  }
}
