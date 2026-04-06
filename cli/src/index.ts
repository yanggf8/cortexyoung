#!/usr/bin/env node

import { loadConfig, saveConfig, requireConfig, configPath, type CortexConfig } from './config.js';
import { applySchema, upsertChunks, upsertProject, deleteStaleProjectChunks, replaceProjectRelationships, projectExists, vectorSearch, keywordSearch, traverseRelationships, getProjectStatus, listProjects, deleteProject, type ChunkRow, type RelationshipRow } from './turso.js';
import { embed, embedBatch, loadModel } from './embedder.js';
import { chunkFile, contextPrefix, type Chunk } from './chunker.js';
import { readFile, readdir, stat } from 'fs/promises';
import { resolve, relative, basename, dirname, extname } from 'path';
import { createHash } from 'crypto';
import { createInterface } from 'readline';
import { execFileSync } from 'child_process';
import { pathToFileURL } from 'url';

const args = process.argv.slice(2);
const command = args[0];

const IGNORE_DIRS = new Set(['node_modules', '.git', '.cortex', 'dist', 'build', '.next', '__pycache__', '.venv', 'venv', 'coverage', '.cache']);
const CODE_EXTS = new Set(['.ts', '.tsx', '.js', '.jsx', '.py', '.go', '.rs', '.java', '.cpp', '.c', '.h', '.md', '.json', '.yaml', '.yml']);
type PendingRelationship = { source_chunk_id: string; source_file_path: string; target_ref: string; rel_type: RelationshipRow['rel_type'] };
type ImportAliasRule = { findPrefix: string; findSuffix: string; replacements: { prefix: string; suffix: string }[] };
type ImportResolver = { projectRoot: string; baseUrl?: string; aliasRules: ImportAliasRule[] };

async function main() {
  switch (command) {
    case 'init': return cmdInit();
    case 'index': return cmdIndex();
    case 'search': return cmdSearch();
    case 'relationships': return cmdRelationships();
    case 'status': return cmdStatus();
    case 'projects': return cmdProjects();
    case 'delete': return cmdDelete();
    case 'config': return cmdConfig();
    default:
      console.log(`Usage: cortex <command>

Commands:
  init                     Set up Turso database and config
  index [path]             Index a directory (default: .)
  search "query"           Semantic search
  search "query" --keyword Keyword search (FTS5)
  relationships "symbol"   Traverse relationships
  status                   Project status
  projects                 List all projects
  delete                   Delete current project
  config                   Show config`);
  }
}

// --- init ---
async function cmdInit() {
  const config = await loadConfig();

  if (config.turso_url && config.turso_auth_token) {
    console.log('Already initialized. Config:', configPath());
    return;
  }

  // Consent prompt
  const answer = await ask('Cortex stores source code chunks (function bodies, class definitions) in Turso cloud database. Continue? [y/N] ');
  if (answer.toLowerCase() !== 'y') {
    console.log('Aborted.');
    return;
  }

  // Check turso CLI
  try {
    execFileSync('turso', ['--version'], { stdio: 'pipe' });
  } catch {
    console.error('turso CLI not found. Install: curl -sSfL https://get.tur.so/install.sh | bash');
    process.exit(1);
  }

  // Create database
  console.log('Creating Turso database...');
  try {
    execFileSync('turso', ['db', 'create', 'cortex-v5'], { stdio: 'inherit' });
  } catch {
    console.log('Database may already exist, continuing...');
  }

  // Get URL and token
  const url = execFileSync('turso', ['db', 'show', 'cortex-v5', '--url'], { encoding: 'utf-8' }).trim();
  const token = execFileSync('turso', ['db', 'tokens', 'create', 'cortex-v5'], { encoding: 'utf-8' }).trim();

  config.turso_url = url;
  config.turso_auth_token = token;
  config.consent_given = true;
  config.binary_path = resolve(process.argv[1]);
  await saveConfig(config);

  // Apply schema
  console.log('Applying schema...');
  await applySchema(config);

  console.log(`Initialized. Config: ${configPath()}`);
  console.log(`Database: ${url}`);
}

// --- index ---
async function cmdIndex() {
  const config = await loadConfig();
  requireConfig(config);

  const targetPath = resolve(args[1] || '.');
  const projectId = createHash('sha256').update(targetPath).digest('hex').slice(0, 16);
  const projectName = basename(targetPath);

  console.log(`Indexing ${targetPath} (project: ${projectName})...`);

  // Collect files
  const files = await collectFiles(targetPath);
  console.log(`Found ${files.length} files`);

  if (files.length === 0) {
    await deleteStaleProjectChunks(config, projectId, []);
    await replaceProjectRelationships(config, projectId, []);
    await upsertProject(config, projectId, projectName, targetPath);
    await setDefaultProject(config, projectId, targetPath);
    console.log('No indexable files found. Existing project chunks were cleared.');
    return;
  }

  // Load embedding model
  console.log('Loading embedding model...');
  await loadModel();

  const importResolver = await loadImportResolver(targetPath);

  // Process files
  let totalChunks = 0;
  const allChunkRows: ChunkRow[] = [];
  const pendingRelationships: PendingRelationship[] = [];
  const symbolIndex = new Map<string, Set<string>>();
  const fileIndex = new Map<string, string>();
  const batchSize = 20; // files per batch

  for (let i = 0; i < files.length; i += batchSize) {
    const fileBatch = files.slice(i, i + batchSize);
    const chunkRows: ChunkRow[] = [];

    for (const filePath of fileBatch) {
      const relPath = relative(targetPath, filePath);
      try {
        const content = await readFile(filePath, 'utf-8');
        if (content.length === 0 || content.length > 100_000) continue; // skip empty/huge files

        const chunks = chunkFile(projectId, relPath, content);

        // Embed all chunks
        const embeddings = await embedBatch(chunks.map(c => contextPrefix(c) + '\n' + c.content));

        for (let j = 0; j < chunks.length; j++) {
          const c = chunks[j];
          indexChunkTarget(symbolIndex, fileIndex, c);
          chunkRows.push({
            chunk_id: c.chunk_id,
            project_id: projectId,
            file_path: c.file_path,
            symbol_name: c.symbol_name,
            chunk_type: c.chunk_type,
            start_line: c.start_line,
            end_line: c.end_line,
            content: c.content,
            content_hash: c.content_hash,
            language: c.language,
            embedding: embeddings[j],
          });

          // Build relationships from chunk analysis
          for (const called of c.calls) {
            pendingRelationships.push({ source_chunk_id: c.chunk_id, source_file_path: c.file_path, target_ref: called, rel_type: 'calls' });
          }
          for (const imp of c.imports) {
            pendingRelationships.push({ source_chunk_id: c.chunk_id, source_file_path: c.file_path, target_ref: imp, rel_type: 'imports' });
          }
          for (const exp of c.exports) {
            pendingRelationships.push({ source_chunk_id: c.chunk_id, source_file_path: c.file_path, target_ref: exp, rel_type: 'exports' });
          }
        }
      } catch (err: any) {
        if (err.code !== 'EISDIR') console.error(`  Skip ${relPath}: ${err.message}`);
      }
    }

    if (chunkRows.length > 0) {
      allChunkRows.push(...chunkRows);
      totalChunks += chunkRows.length;
      process.stdout.write(`\r  Indexed ${totalChunks} chunks from ${Math.min(i + batchSize, files.length)}/${files.length} files`);
    }
  }

  const resolvedRelationships = resolveRelationships(pendingRelationships, symbolIndex, fileIndex, importResolver);
  const currentChunkIds = allChunkRows.map(chunk => chunk.chunk_id);

  if (allChunkRows.length > 0) {
    await upsertChunks(config, allChunkRows);
  }
  await deleteStaleProjectChunks(config, projectId, currentChunkIds);
  await replaceProjectRelationships(config, projectId, resolvedRelationships);

  await upsertProject(config, projectId, projectName, targetPath);
  await setDefaultProject(config, projectId, targetPath);

  console.log(`\nDone. ${totalChunks} chunks indexed.`);

  if (hasFlag('--format', 'json')) {
    console.log(JSON.stringify({ project_id: projectId, chunks: totalChunks }));
  }
}

// --- search ---
async function cmdSearch() {
  const config = await loadConfig();
  requireConfig(config);

  const query = args[1];
  if (!query) { console.error('Usage: cortex search "query"'); process.exit(1); }

  const projectId = await getProjectId(config);
  const topK = parseInt(getFlag('--top-k') || '15');
  const offset = parseInt(getFlag('--offset') || '0');

  if (hasFlag('--keyword')) {
    const result = await keywordSearch(config, query, projectId, topK, offset);
    output(result);
  } else {
    console.error('Loading model...');
    const vector = await embed(query);
    const result = await vectorSearch(config, vector, projectId, topK, offset);
    output(result);
  }
}

// --- relationships ---
async function cmdRelationships() {
  const config = await loadConfig();
  requireConfig(config);

  const symbol = args[1];
  if (!symbol) { console.error('Usage: cortex relationships "symbol"'); process.exit(1); }

  const projectId = await getProjectId(config);
  const depth = parseInt(getFlag('--depth') || '2');
  const result = await traverseRelationships(config, projectId, symbol, depth);
  output(result);
}

// --- status ---
async function cmdStatus() {
  const config = await loadConfig();
  requireConfig(config);

  const projectId = await getProjectId(config);
  const status = await getProjectStatus(config, projectId);
  if (!status) { console.error('Project not found. Run: cortex index'); process.exit(1); }
  output(status);
}

// --- projects ---
async function cmdProjects() {
  const config = await loadConfig();
  requireConfig(config);

  const projects = await listProjects(config);
  output({ projects });
}

// --- delete ---
async function cmdDelete() {
  const config = await loadConfig();
  requireConfig(config);

  const projectId = await getProjectId(config);
  const deleted = await deleteProject(config, projectId);
  if (config.default_project_id === projectId) {
    config.default_project_id = '';
    config.default_project_path = '';
    await saveConfig(config);
  }
  output({ deleted_chunks: deleted, project_id: projectId });
}

// --- config ---
async function cmdConfig() {
  const config = await loadConfig();
  console.log(`Config: ${configPath()}`);
  console.log(JSON.stringify({ ...config, turso_auth_token: config.turso_auth_token ? '***' : '' }, null, 2));
}

// --- helpers ---

async function collectFiles(dir: string): Promise<string[]> {
  const files: string[] = [];

  async function walk(d: string) {
    const entries = await readdir(d, { withFileTypes: true });
    for (const entry of entries) {
      if (entry.name.startsWith('.') && entry.name !== '.') continue;
      if (IGNORE_DIRS.has(entry.name)) continue;

      const full = resolve(d, entry.name);
      if (entry.isDirectory()) {
        await walk(full);
      } else if (entry.isFile()) {
        const e = full.substring(full.lastIndexOf('.')).toLowerCase();
        if (CODE_EXTS.has(e)) files.push(full);
      }
    }
  }

  await walk(dir);
  return files;
}

async function getProjectId(config: CortexConfig): Promise<string> {
  const explicit = getFlag('--project');
  if (explicit) return explicit;

  const cwd = resolve('.');
  const cwdProjectId = hashProjectPath(cwd);
  return resolvePreferredProjectId(
    cwdProjectId,
    await projectExists(config, cwdProjectId),
    config.default_project_id
  );
}

async function setDefaultProject(config: CortexConfig, projectId: string, projectPath: string): Promise<void> {
  config.default_project_id = projectId;
  config.default_project_path = projectPath;
  await saveConfig(config);
}

function indexChunkTarget(symbolIndex: Map<string, Set<string>>, fileIndex: Map<string, string>, chunk: Chunk): void {
  if (chunk.symbol_name) {
    let ids = symbolIndex.get(chunk.symbol_name);
    if (!ids) {
      ids = new Set<string>();
      symbolIndex.set(chunk.symbol_name, ids);
    }
    ids.add(chunk.chunk_id);
  }

  const fileKeys = getFileKeys(chunk.file_path);
  for (const key of fileKeys) {
    if (!fileIndex.has(key)) fileIndex.set(key, chunk.chunk_id);
  }
}

function resolveRelationships(
  pendingRelationships: PendingRelationship[],
  symbolIndex: Map<string, Set<string>>,
  fileIndex: Map<string, string>,
  importResolver: ImportResolver
): RelationshipRow[] {
  const deduped = new Map<string, RelationshipRow>();

  for (const rel of pendingRelationships) {
    const targetIds = rel.rel_type === 'imports'
      ? resolveImportTargets(rel.target_ref, rel.source_file_path, fileIndex, importResolver)
      : [...(symbolIndex.get(rel.target_ref) ?? [])];

    for (const targetChunkId of targetIds) {
      const key = `${rel.source_chunk_id}:${targetChunkId}:${rel.rel_type}`;
      deduped.set(key, {
        source_chunk_id: rel.source_chunk_id,
        target_chunk_id: targetChunkId,
        rel_type: rel.rel_type,
      });
    }
  }

  return [...deduped.values()];
}

function resolveImportTargets(
  specifier: string,
  sourceFilePath: string,
  fileIndex: Map<string, string>,
  importResolver: ImportResolver
): string[] {
  const rawCandidates = resolveImportCandidates(specifier, sourceFilePath, importResolver);

  const targetIds = new Set<string>();
  for (const candidate of rawCandidates) {
    for (const key of getFileKeys(candidate)) {
      const targetId = fileIndex.get(key);
      if (targetId) targetIds.add(targetId);
    }
  }
  return [...targetIds];
}

function resolveImportCandidates(specifier: string, sourceFilePath: string, importResolver: ImportResolver): string[] {
  const candidates = new Set<string>();

  if (specifier.startsWith('.')) {
    const sourceDir = dirname(sourceFilePath);
    const resolvedSpecifier = relative(importResolver.projectRoot, resolve(importResolver.projectRoot, sourceDir, specifier)).replace(/\\/g, '/');
    candidates.add(resolvedSpecifier);
    candidates.add(stripExtension(resolvedSpecifier));
    candidates.add(`${stripExtension(resolvedSpecifier)}/index`);
    return [...candidates];
  }

  for (const rule of importResolver.aliasRules) {
    const matched = matchAliasRule(specifier, rule);
    if (!matched) continue;

    for (const replacement of rule.replacements) {
      const replaced = `${replacement.prefix}${matched}${replacement.suffix}`.replace(/\\/g, '/');
      candidates.add(replaced);
      candidates.add(stripExtension(replaced));
      candidates.add(`${stripExtension(replaced)}/index`);
    }
  }

  if (importResolver.baseUrl) {
    const resolvedSpecifier = relative(
      importResolver.projectRoot,
      resolve(importResolver.projectRoot, importResolver.baseUrl, specifier)
    ).replace(/\\/g, '/');
    candidates.add(resolvedSpecifier);
    candidates.add(stripExtension(resolvedSpecifier));
    candidates.add(`${stripExtension(resolvedSpecifier)}/index`);
  }

  return [...candidates];
}

function getFileKeys(filePath: string): string[] {
  const normalized = filePath.replace(/\\/g, '/');
  const withoutExt = stripExtension(normalized);
  const keys = [normalized, withoutExt];

  if (!normalized.includes('/')) {
    const base = basename(normalized);
    keys.push(base, stripExtension(base));
  }

  return [...new Set(keys)];
}

function stripExtension(value: string): string {
  const extension = extname(value);
  return extension ? value.slice(0, -extension.length) : value;
}

export function hashProjectPath(projectPath: string): string {
  return createHash('sha256').update(projectPath).digest('hex').slice(0, 16);
}

export function resolvePreferredProjectId(
  cwdProjectId: string,
  cwdProjectExists: boolean,
  defaultProjectId: string
): string {
  if (cwdProjectExists) return cwdProjectId;
  if (defaultProjectId) return defaultProjectId;
  return cwdProjectId;
}

export function resolveRelationshipsForTest(
  pendingRelationships: PendingRelationship[],
  symbolIndex: Map<string, Set<string>>,
  fileIndex: Map<string, string>,
  importResolver: ImportResolver = { projectRoot: '.', aliasRules: [] }
): RelationshipRow[] {
  return resolveRelationships(pendingRelationships, symbolIndex, fileIndex, importResolver);
}

export function resolveImportTargetsForTest(
  specifier: string,
  sourceFilePath: string,
  fileIndex: Map<string, string>,
  importResolver: ImportResolver = { projectRoot: '.', aliasRules: [] }
): string[] {
  return resolveImportTargets(specifier, sourceFilePath, fileIndex, importResolver);
}

export function getFileKeysForTest(filePath: string): string[] {
  return getFileKeys(filePath);
}

export function createImportResolverForTest(projectRoot: string, baseUrl?: string, aliasRules: ImportAliasRule[] = []): ImportResolver {
  return { projectRoot, baseUrl, aliasRules };
}

async function loadImportResolver(projectRoot: string): Promise<ImportResolver> {
  const configPaths = [resolve(projectRoot, 'tsconfig.json'), resolve(projectRoot, 'jsconfig.json')];

  for (const configPath of configPaths) {
    try {
      const raw = await readFile(configPath, 'utf-8');
      const parsed = JSON.parse(stripJsonComments(raw));
      const compilerOptions = parsed.compilerOptions ?? {};
      const baseUrl = typeof compilerOptions.baseUrl === 'string' ? compilerOptions.baseUrl : undefined;
      const aliasRules = parseAliasRules(compilerOptions.paths ?? {});
      return { projectRoot, baseUrl, aliasRules };
    } catch {
      // Ignore missing or invalid config files and fall back to relative-only resolution.
    }
  }

  return { projectRoot, aliasRules: [] };
}

function parseAliasRules(pathsConfig: Record<string, unknown>): ImportAliasRule[] {
  const rules: ImportAliasRule[] = [];

  for (const [pattern, rawTargets] of Object.entries(pathsConfig)) {
    if (!Array.isArray(rawTargets)) continue;

    const replacements = rawTargets
      .filter((value): value is string => typeof value === 'string')
      .map(value => ({
        prefix: value.includes('*') ? value.slice(0, value.indexOf('*')) : value,
        suffix: value.includes('*') ? value.slice(value.indexOf('*') + 1) : '',
      }));

    if (replacements.length === 0) continue;

    rules.push({
      findPrefix: pattern.includes('*') ? pattern.slice(0, pattern.indexOf('*')) : pattern,
      findSuffix: pattern.includes('*') ? pattern.slice(pattern.indexOf('*') + 1) : '',
      replacements,
    });
  }

  return rules;
}

function matchAliasRule(specifier: string, rule: ImportAliasRule): string | null {
  if (!specifier.startsWith(rule.findPrefix)) return null;
  if (!specifier.endsWith(rule.findSuffix)) return null;

  const start = rule.findPrefix.length;
  const end = specifier.length - rule.findSuffix.length;
  return specifier.slice(start, end);
}

function stripJsonComments(value: string): string {
  return value
    .replace(/\/\*[\s\S]*?\*\//g, '')
    .replace(/^\s*\/\/.*$/gm, '');
}

function getFlag(name: string): string | undefined {
  const idx = args.indexOf(name);
  return idx >= 0 && idx + 1 < args.length ? args[idx + 1] : undefined;
}

function hasFlag(...names: string[]): boolean {
  return names.some(n => args.includes(n));
}

function output(data: any): void {
  console.log(JSON.stringify(data, null, hasFlag('--format', 'json') ? undefined : 2));
}

function ask(question: string): Promise<string> {
  const rl = createInterface({ input: process.stdin, output: process.stdout });
  return new Promise(resolve => rl.question(question, answer => { rl.close(); resolve(answer); }));
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  main().catch(err => {
    console.error(err.message || err);
    process.exit(1);
  });
}
