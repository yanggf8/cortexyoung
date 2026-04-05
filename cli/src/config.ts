import { readFile, writeFile, mkdir } from 'fs/promises';
import { existsSync } from 'fs';
import { homedir } from 'os';
import { join } from 'path';

export interface CortexConfig {
  turso_url: string;
  turso_auth_token: string;
  binary_path: string;
  consent_given: boolean;
  model: string;
  dimensions: number;
  default_project_id: string;
  default_project_path: string;
}

const CONFIG_DIR = join(homedir(), '.cortex');
const CONFIG_PATH = join(CONFIG_DIR, 'config.json');

const DEFAULT_CONFIG: CortexConfig = {
  turso_url: '',
  turso_auth_token: '',
  binary_path: '',
  consent_given: false,
  model: 'Xenova/bge-small-en-v1.5',
  dimensions: 384,
  default_project_id: '',
  default_project_path: '',
};

export async function loadConfig(): Promise<CortexConfig> {
  if (!existsSync(CONFIG_PATH)) return { ...DEFAULT_CONFIG };
  const raw = await readFile(CONFIG_PATH, 'utf-8');
  return { ...DEFAULT_CONFIG, ...JSON.parse(raw) };
}

export async function saveConfig(config: CortexConfig): Promise<void> {
  if (!existsSync(CONFIG_DIR)) await mkdir(CONFIG_DIR, { recursive: true });
  await writeFile(CONFIG_PATH, JSON.stringify(config, null, 2), { mode: 0o600 });
}

export function configPath(): string {
  return CONFIG_PATH;
}

export function requireConfig(config: CortexConfig): void {
  if (!config.turso_url || !config.turso_auth_token) {
    console.error('Cortex not initialized. Run: cortex init');
    process.exit(1);
  }
}
