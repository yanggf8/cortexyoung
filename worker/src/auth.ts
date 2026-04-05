// Auth middleware: validate API keys against Turso api_keys table
import type { Env } from './types';
import { getTursoClient } from './turso-client';

export async function authenticate(
  request: Request,
  env: Env
): Promise<{ authenticated: boolean; error?: string }> {
  const authHeader = request.headers.get('Authorization');
  if (!authHeader || !authHeader.startsWith('Bearer ')) {
    return { authenticated: false, error: 'Missing Authorization header' };
  }

  const apiKey = authHeader.slice(7).trim();
  if (!apiKey) {
    return { authenticated: false, error: 'Empty API key' };
  }

  // Look up key in Turso
  const db = getTursoClient(env);
  const result = await db.execute({
    sql: 'SELECT active FROM api_keys WHERE api_key = ? AND active = 1',
    args: [apiKey],
  });

  if (result.rows.length === 0) {
    return { authenticated: false, error: 'Invalid API key' };
  }

  return { authenticated: true };
}

// Register a new API key (called during cortex init)
export async function registerKey(
  apiKey: string,
  env: Env
): Promise<void> {
  const db = getTursoClient(env);
  await db.execute({
    sql: 'INSERT OR IGNORE INTO api_keys (api_key) VALUES (?)',
    args: [apiKey],
  });
}

// Delete an API key (for rotation)
export async function revokeKey(
  apiKey: string,
  env: Env
): Promise<void> {
  const db = getTursoClient(env);
  await db.execute({
    sql: 'DELETE FROM api_keys WHERE api_key = ?',
    args: [apiKey],
  });
}

// Validate setup token for admin routes
export function validateSetupToken(
  request: Request,
  env: Env
): boolean {
  const authHeader = request.headers.get('Authorization');
  if (!authHeader || !authHeader.startsWith('Bearer ')) return false;
  return authHeader.slice(7).trim() === env.SETUP_TOKEN;
}
