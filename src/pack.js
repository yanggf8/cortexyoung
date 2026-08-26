import { createHash } from 'node:crypto';
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

export const PACK_DIR = fileURLToPath(new URL('./pack', import.meta.url));
export const SGCONFIG = path.join(PACK_DIR, 'sgconfig.yml');

export function packFiles() {
  const out = [];
  const walk = (dir) => {
    for (const entry of fs.readdirSync(dir, { withFileTypes: true })) {
      const p = path.join(dir, entry.name);
      if (entry.isDirectory()) walk(p);
      else if (entry.isFile() && p.endsWith('.yml')) out.push(p);
    }
  };
  walk(PACK_DIR);
  return out.sort();
}

export function extractorVersion() {
  const h = createHash('sha256');
  for (const f of packFiles()) h.update(fs.readFileSync(f));
  return h.digest('hex');
}
