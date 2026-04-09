/**
 * Grammar management for web-tree-sitter WASM grammars.
 *
 * Resolution order:
 * 1. npm packages (tree-sitter-javascript, etc.) — zero download, ships with install
 * 2. ~/.cortex/grammars/ cache — for offline installs or custom grammars
 */

import { readFile, writeFile, readdir, mkdir, copyFile, stat } from 'fs/promises';
import { resolve, dirname } from 'path';
import { createHash } from 'crypto';
import { homedir } from 'os';
import { createRequire } from 'module';

export interface GrammarInfo {
  name: string;
  extensions: string[];
  /** WASM file name */
  wasmFile: string;
  /** npm package that ships this WASM file */
  npmPackage: string;
  /** Path within the npm package to the .wasm file */
  npmWasmPath: string;
}

const GRAMMARS_DIR = resolve(homedir(), '.cortex', 'grammars');

const GRAMMAR_REGISTRY: GrammarInfo[] = [
  {
    name: 'javascript',
    extensions: ['.js', '.jsx'],
    wasmFile: 'tree-sitter-javascript.wasm',
    npmPackage: 'tree-sitter-javascript',
    npmWasmPath: 'tree-sitter-javascript.wasm',
  },
  {
    name: 'typescript',
    extensions: ['.ts'],
    wasmFile: 'tree-sitter-typescript.wasm',
    npmPackage: 'tree-sitter-typescript',
    npmWasmPath: 'tree-sitter-typescript.wasm',
  },
  {
    name: 'tsx',
    extensions: ['.tsx'],
    wasmFile: 'tree-sitter-tsx.wasm',
    npmPackage: 'tree-sitter-typescript',
    npmWasmPath: 'tree-sitter-tsx.wasm',
  },
  {
    name: 'python',
    extensions: ['.py'],
    wasmFile: 'tree-sitter-python.wasm',
    npmPackage: 'tree-sitter-python',
    npmWasmPath: 'tree-sitter-python.wasm',
  },
];

const EXT_TO_GRAMMAR = new Map<string, GrammarInfo>();
for (const g of GRAMMAR_REGISTRY) {
  for (const ext of g.extensions) {
    EXT_TO_GRAMMAR.set(ext, g);
  }
}

export function grammarForExtension(ext: string): GrammarInfo | undefined {
  return EXT_TO_GRAMMAR.get(ext);
}

export function grammarsDir(): string {
  return GRAMMARS_DIR;
}

export function allGrammars(): GrammarInfo[] {
  return [...GRAMMAR_REGISTRY];
}

// --- WASM resolution ---

const require = createRequire(import.meta.url);

/** Resolve the WASM file path for a grammar. Tries npm package first, then cache dir. */
export function resolveGrammarWasm(grammar: GrammarInfo): string | null {
  // Try npm package first
  try {
    const pkgJsonPath = require.resolve(`${grammar.npmPackage}/package.json`);
    const pkgDir = dirname(pkgJsonPath);
    const wasmPath = resolve(pkgDir, grammar.npmWasmPath);
    // We'll check existence synchronously via try-catch on require.resolve
    // but .wasm files aren't resolvable by require, so use the computed path
    return wasmPath;
  } catch {
    // npm package not installed
  }

  // Try cache directory
  const cachePath = resolve(GRAMMARS_DIR, grammar.wasmFile);
  return cachePath;
}

/** Check if a grammar WASM file is available (either from npm or cache) */
export async function isGrammarAvailable(grammar: GrammarInfo): Promise<boolean> {
  // Try npm package
  try {
    const pkgJsonPath = require.resolve(`${grammar.npmPackage}/package.json`);
    const pkgDir = dirname(pkgJsonPath);
    const wasmPath = resolve(pkgDir, grammar.npmWasmPath);
    await stat(wasmPath);
    return true;
  } catch {
    // Not in npm
  }

  // Try cache
  try {
    await stat(resolve(GRAMMARS_DIR, grammar.wasmFile));
    return true;
  } catch {
    return false;
  }
}

/** Get the resolved WASM path, checking both npm and cache. Returns null if not found. */
export async function grammarPath(grammar: GrammarInfo): Promise<string | null> {
  // Try npm package first
  try {
    const pkgJsonPath = require.resolve(`${grammar.npmPackage}/package.json`);
    const pkgDir = dirname(pkgJsonPath);
    const wasmPath = resolve(pkgDir, grammar.npmWasmPath);
    await stat(wasmPath);
    return wasmPath;
  } catch {
    // Not in npm
  }

  // Try cache
  const cachePath = resolve(GRAMMARS_DIR, grammar.wasmFile);
  try {
    await stat(cachePath);
    return cachePath;
  } catch {
    return null;
  }
}

/** Ensure the grammars directory exists */
async function ensureDir(): Promise<void> {
  await mkdir(GRAMMARS_DIR, { recursive: true });
}

/** Ensure all grammars needed for a set of extensions are available */
export async function ensureGrammars(extensions: Set<string>): Promise<{ available: string[]; missing: string[] }> {
  const needed = new Set<GrammarInfo>();
  for (const ext of extensions) {
    const g = grammarForExtension(ext);
    if (g) needed.add(g);
  }

  const available: string[] = [];
  const missing: string[] = [];

  for (const grammar of needed) {
    if (await isGrammarAvailable(grammar)) {
      available.push(grammar.name);
    } else {
      missing.push(grammar.name);
    }
  }

  return { available, missing };
}

/** Install grammars from a local directory containing .wasm files */
export async function installFromLocal(sourcePath: string): Promise<string[]> {
  await ensureDir();
  const installed: string[] = [];

  try {
    const entries = await readdir(sourcePath);
    for (const entry of entries) {
      if (!entry.endsWith('.wasm')) continue;
      const grammar = GRAMMAR_REGISTRY.find(g => g.wasmFile === entry);
      if (!grammar) continue;

      await copyFile(resolve(sourcePath, entry), resolve(GRAMMARS_DIR, grammar.wasmFile));
      installed.push(grammar.name);
    }
  } catch {
    throw new Error(`Cannot read grammar source: ${sourcePath}`);
  }

  return installed;
}

/** Compute a version hash over all available grammar WASM files */
export async function computeGrammarVersionHash(): Promise<string> {
  const hash = createHash('sha256');
  let hasAny = false;

  for (const grammar of GRAMMAR_REGISTRY) {
    const wasmPath = await grammarPath(grammar);
    if (!wasmPath) continue;

    try {
      const data = await readFile(wasmPath);
      hash.update(grammar.name);
      hash.update(data);
      hasAny = true;
    } catch {
      // Grammar not available — skip
    }
  }

  return hasAny ? hash.digest('hex').slice(0, 16) : '';
}

/** Get status of all grammars */
export async function grammarStatus(): Promise<{ name: string; available: boolean; source: string; path: string }[]> {
  return Promise.all(
    GRAMMAR_REGISTRY.map(async g => {
      const wasmPath = await grammarPath(g);
      const available = wasmPath !== null;
      let source = 'not installed';
      if (available && wasmPath) {
        source = wasmPath.includes('node_modules') ? 'npm' : 'cache';
      }
      return { name: g.name, available, source, path: wasmPath || resolve(GRAMMARS_DIR, g.wasmFile) };
    })
  );
}
