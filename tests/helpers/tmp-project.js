import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';

export function makeProject(files) {
  const root = fs.realpathSync(fs.mkdtempSync(path.join(os.tmpdir(), 'cort-proj-')));
  for (const [rel, body] of Object.entries(files)) {
    const abs = path.join(root, rel);
    fs.mkdirSync(path.dirname(abs), { recursive: true });
    fs.writeFileSync(abs, body);
  }
  return root;
}

export const SAMPLE = {
  'src/helper.ts': 'export function helper(n: number) { return n * 2; }\n',
  'src/alpha.ts': [
    "import { helper } from './helper';",
    'export function alpha(a: number) { return helper(a) + 1; }',
    'export class Beta {',
    '  go() { return alpha(2); }',
    '}',
  ].join('\n') + '\n',
  'node_modules/pkg/index.ts': 'export function shouldBeIgnored() {}\n',
  'README.md': '# not a source file\n',
};
