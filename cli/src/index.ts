#!/usr/bin/env node

import { loadConfig, saveConfig, requireConfig, configPath, type CortexConfig } from './config.js';
import { applySchema, upsertChunks, upsertProject, deleteStaleProjectChunks, replaceProjectRelationships, replaceFileRelationships, deleteFileChunks, deleteStaleFileChunks, getIndexedFilePaths, projectExists, vectorSearch, keywordSearch, hybridSearch, traverseRelationships, getDirectNeighbors, getTransitiveDependents, findChunksBySymbol, findChunksByFile, getProjectStatus, getProjectMeta, listProjects, deleteProject, getProjectGraphData, sanitizeFtsQuery, type ChunkRow, type RelationshipRow, type RelationshipConfidence, type SearchFilters } from './turso.js';
import { clusterProject } from './clusterer.js';
import { embed, embedBatch, loadModel } from './embedder.js';
import { chunkFile, chunkFileAST, contextPrefix, type Chunk } from './chunker.js';
import { initParser } from './ast-chunker.js';
import { ensureGrammars, computeGrammarVersionHash, grammarStatus, installFromLocal, grammarsDir } from './grammars.js';
import { readFile, writeFile, readdir, stat, watch } from 'fs/promises';
import { resolve, relative, basename, dirname, extname } from 'path';
import { createHash } from 'crypto';
import { createInterface } from 'readline';
import { execFileSync } from 'child_process';
import { pathToFileURL } from 'url';

const args = process.argv.slice(2);
const command = args[0];

const IGNORE_DIRS = new Set(['node_modules', '.git', '.cortex', 'dist', 'build', '.next', '__pycache__', '.venv', 'venv', 'coverage', '.cache']);
const IGNORE_FILES = new Set(['CORTEX_REPORT.md', 'package-lock.json', 'yarn.lock', 'pnpm-lock.yaml']);
const CODE_EXTS = new Set(['.ts', '.tsx', '.js', '.jsx', '.py', '.go', '.rs', '.java', '.cpp', '.c', '.h', '.md', '.json', '.yaml', '.yml']);
type PendingRelationship = { source_chunk_id: string; source_file_path: string; target_ref: string; rel_type: RelationshipRow['rel_type'] };
type ImportAliasRule = { findPrefix: string; findSuffix: string; replacements: { prefix: string; suffix: string }[] };
type ImportResolver = { projectRoot: string; baseUrl?: string; aliasRules: ImportAliasRule[] };

async function main() {
  switch (command) {
    case 'init': return cmdInit();
    case 'index': return cmdIndex();
    case 'search': return cmdSearch();
    case 'context': return cmdContext();
    case 'impact': return cmdImpact();
    case 'relationships': return cmdRelationships();
    case 'status': return cmdStatus();
    case 'projects': return cmdProjects();
    case 'delete': return cmdDelete();
    case 'config': return cmdConfig();
    case 'grammars': return cmdGrammars();
    case 'modules': return cmdModules();
    default:
      console.log(`Usage: cortex <command>

Commands:
  init                       Set up Turso database and config
  index [path]               Full index of a directory (default: .)
  index [path] --watch       Index then watch for changes
  index [path] --incremental Reindex only files changed since last run
  search "query"             Hybrid search (vector + FTS, RRF fusion)
  search "query" --vector    Vector-only semantic search
  search "query" --keyword   Keyword-only search (FTS5)
  context "symbol-or-query"  Minimal context pack for a symbol or query
  impact --symbol "name"     Blast-radius analysis for a symbol
  impact --from-diff [sha]   Impact analysis from git diff
  relationships "symbol"     Traverse relationships
  status                     Project status
  projects                   List all projects
  delete                     Delete current project
  config                     Show config
  modules                    Detect subsystem modules via Louvain clustering
  modules --min-size N       Only show modules with ≥ N files (default: 1)
  grammars                   Show grammar status
  grammars install <path>    Install grammars from local directory

Flags:
  --quiet                    Suppress staleness hint on search/status
  --rrf-k N                  RRF smoothing constant (default: 60)
  CORTEX_QUIET=1             (env) Suppress staleness hint globally`);
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

// Ensure the Turso schema (including ALTER TABLE migrations for newly added
// columns) is applied before any read or write. Memoized per process so read
// commands pay the cost at most once. Read commands must call this because
// the P4 migration added `confidence_score` / `confidence_reasoning` columns
// that are SELECTed unconditionally in turso.ts — without this, upgrading
// users would hit "no such column" until they manually reran `cortex index`.
let schemaEnsured = false;
async function ensureSchema(config: CortexConfig): Promise<void> {
  if (schemaEnsured) return;
  await applySchema(config);
  schemaEnsured = true;
}

// --- P5: AST-aware query filters ---
// SearchFilters interface lives in turso.ts (shared with the SQL layer).

const LANG_ALIAS: Record<string, string> = {
  ts: 'typescript', tsx: 'typescript', typescript: 'typescript',
  js: 'javascript', jsx: 'javascript', javascript: 'javascript',
  py: 'python', python: 'python',
  go: 'go', rs: 'rust', rust: 'rust',
  java: 'java', cpp: 'cpp', c: 'c',
  md: 'markdown', markdown: 'markdown',
};

// `interface` chunks are stored as 'config' by the AST chunker — alias for usability.
const KIND_ALIAS: Record<string, string> = {
  method: 'function',
  interface: 'config',
  type: 'config',
  enum: 'config',
};

/** Convert a user glob (`*`, `**`) to a SQL LIKE pattern. */
function globToLike(glob: string): string {
  // Escape SQL LIKE wildcards already present in the user input
  const escaped = glob.replace(/[\\%_]/g, c => '\\' + c);
  // Then convert glob `*`/`**` to SQL `%`
  return escaped.replace(/\*+/g, '%');
}

/**
 * Parse `kind:`, `lang:`, `name:`, `file:` filter tokens out of a free-form
 * search query. Returns the filters and the remaining text query.
 *
 * Examples:
 *   "useEffect kind:function lang:ts" → { textQuery: "useEffect", filters: {kind:'function',language:'typescript'} }
 *   "name:parse* file:src/auth/**"    → { textQuery: "",          filters: {symbolGlob:'parse%',fileGlob:'src/auth/%'} }
 */
export function parseQueryFilters(query: string): { textQuery: string; filters: SearchFilters } {
  const filters: SearchFilters = {};
  const remaining: string[] = [];

  // Split on whitespace, preserving quoted phrases.
  const tokens = query.match(/(?:[^\s"]+|"[^"]*")+/g) ?? [];

  for (const tok of tokens) {
    const m = tok.match(/^(kind|lang|name|file):(.+)$/);
    if (!m) {
      remaining.push(tok);
      continue;
    }
    const [, field, rawValue] = m;
    // Strip surrounding quotes if present
    const value = rawValue.replace(/^"(.*)"$/, '$1');

    if (field === 'kind') {
      filters.kind = KIND_ALIAS[value.toLowerCase()] ?? value.toLowerCase();
    } else if (field === 'lang') {
      filters.language = LANG_ALIAS[value.toLowerCase()] ?? value.toLowerCase();
    } else if (field === 'name') {
      filters.symbolGlob = globToLike(value);
    } else if (field === 'file') {
      filters.fileGlob = globToLike(value);
    }
  }

  return { textQuery: remaining.join(' ').trim(), filters };
}

/** True if the SearchFilters object has any active filter. */
export function hasFilters(f: SearchFilters): boolean {
  return f.kind != null || f.language != null || f.symbolGlob != null || f.fileGlob != null;
}

// --- index ---
async function cmdIndex() {
  const config = await loadConfig();
  requireConfig(config);

  // Ensure schema is up-to-date (migrates existing DBs for new columns)
  await ensureSchema(config);

  const targetPath = resolve(args[1] || '.');
  const projectId = createHash('sha256').update(targetPath).digest('hex').slice(0, 16);
  const projectName = basename(targetPath);

  // Incremental mode: diff against stored git sha / mtime, reindex only changed files.
  if (hasFlag('--incremental')) {
    const ran = await cmdIndexIncremental(config, targetPath, projectId, projectName);
    if (ran) return;
    console.log('Incremental not possible — falling back to full index.');
  }

  console.log(`Indexing ${targetPath} (project: ${projectName})...`);

  // Collect files
  const files = await collectFiles(targetPath);
  console.log(`Found ${files.length} files`);

  const gitHead = getGitHead(targetPath);
  const indexStartedAt = Date.now();

  if (files.length === 0) {
    await deleteStaleProjectChunks(config, projectId, []);
    await replaceProjectRelationships(config, projectId, []);
    await upsertProject(config, projectId, projectName, targetPath, {
      gitHead: gitHead ?? undefined,
      lastIndexedAt: indexStartedAt,
    });
    await setDefaultProject(config, projectId, targetPath);
    console.log('No indexable files found. Existing project chunks were cleared.');
    return;
  }

  // Load embedding model + AST parser in parallel
  console.log('Loading embedding model...');
  const fileExts = new Set(files.map(f => f.substring(f.lastIndexOf('.')).toLowerCase()));
  const [, , grammarResult] = await Promise.all([
    loadModel(),
    initParser(),
    ensureGrammars(fileExts),
  ]);
  const grammarVersion = await computeGrammarVersionHash();
  const astEnabled = grammarResult.available.length > 0;
  if (astEnabled) {
    console.log(`AST chunking enabled (${grammarResult.available.join(', ')})`);
    if (grammarResult.missing.length > 0) console.log(`Grammars not found: ${grammarResult.missing.join(', ')} (regex fallback)`);
  } else {
    console.log('AST chunking not available (no grammars), using regex');
  }

  const importResolver = await loadImportResolver(targetPath);

  // Process files
  let totalChunks = 0;
  let astChunks = 0;
  let regexChunks = 0;
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

        const chunks = astEnabled
          ? await chunkFileAST(projectId, relPath, content)
          : chunkFile(projectId, relPath, content);

        // Track AST vs regex usage
        for (const c of chunks) {
          if (c.chunk_source === 'ast') astChunks++;
          else regexChunks++;
        }

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
            chunk_source: c.chunk_source,
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

  await upsertProject(config, projectId, projectName, targetPath, {
    grammarVersion: grammarVersion || undefined,
    gitHead: gitHead ?? undefined,
    lastIndexedAt: indexStartedAt,
  });
  await setDefaultProject(config, projectId, targetPath);

  const sourceBreakdown = astChunks > 0 ? ` (AST: ${astChunks}, regex: ${regexChunks})` : '';
  console.log(`\nDone. ${totalChunks} chunks indexed${sourceBreakdown}.`);
  if (gitHead) {
    console.log(`Indexed at: ${gitHead.slice(0, 12)}`);
  }

  // Generate report (best-effort — read-only trees shouldn't fail the whole index)
  try {
    await generateReport(targetPath, projectName, projectId, allChunkRows, resolvedRelationships, gitHead, indexStartedAt);
  } catch (err: any) {
    console.error(`  Warning: could not write CORTEX_REPORT.md: ${err.message}`);
  }

  if (hasFlag('--format', 'json')) {
    console.log(JSON.stringify({ project_id: projectId, chunks: totalChunks }));
  }

  // Watch mode: monitor for changes and re-index incrementally
  if (hasFlag('--watch')) {
    await watchAndReindex(config, targetPath, projectId, projectName, importResolver);
  }
}

/**
 * Incremental reindex path: uses git diff (or mtime fallback) to find changed files,
 * then replays each through the per-file CRUD path. Returns false when incremental
 * is not possible (no prior index, stored sha missing, nothing to do delegation).
 */
async function cmdIndexIncremental(
  config: CortexConfig,
  targetPath: string,
  projectId: string,
  projectName: string,
): Promise<boolean> {
  const meta = await getProjectMeta(config, projectId);
  if (!meta) {
    console.log('No prior index found. Run `cortex index` without --incremental first.');
    return false;
  }

  const currentHead = getGitHead(targetPath);
  const isGit = currentHead !== null;

  // Collect current files + indexed file set for deletion detection
  const allFiles = await collectFiles(targetPath);
  const indexedPaths = await getIndexedFilePaths(config, projectId);
  const plan = await computeIncrementalChanges(targetPath, meta.git_head, meta.last_indexed_at, allFiles, indexedPaths);

  if (plan.method === 'full') {
    console.log('No git sha and no prior timestamp to diff against — cannot run incremental.');
    return false;
  }

  if (plan.changes.length === 0) {
    console.log(`Nothing to reindex (${plan.method} diff found no changes).`);
    // Still bump stored metadata so next run has a fresh baseline
    const grammarVersion = await computeGrammarVersionHash();
    await upsertProject(config, projectId, projectName, targetPath, {
      grammarVersion: grammarVersion || undefined,
      gitHead: currentHead ?? meta.git_head ?? undefined,
      lastIndexedAt: Date.now(),
    });
    return true;
  }

  console.log(`Incremental reindex (${plan.method}): ${plan.changes.length} changed file(s)`);

  // Load embedding model + parser + grammars (same as full index)
  const fileExts = new Set(allFiles.map(f => f.substring(f.lastIndexOf('.')).toLowerCase()));
  const [, , grammarResult] = await Promise.all([
    loadModel(),
    initParser(),
    ensureGrammars(fileExts),
  ]);
  const grammarVersion = await computeGrammarVersionHash();
  if (grammarResult.available.length === 0) {
    console.log('AST chunking not available (no grammars), using regex');
  }

  const importResolver = await loadImportResolver(targetPath);

  let reindexed = 0;
  let deleted = 0;
  let failed = 0;

  for (const change of plan.changes) {
    // Renames: delete the old path first, then index the new path
    if (change.status === 'R' && change.oldPath) {
      const removed = await deleteFileChunks(config, projectId, change.oldPath);
      if (removed > 0) deleted++;
    }

    if (change.status === 'D') {
      const removed = await deleteFileChunks(config, projectId, change.path);
      if (removed > 0) {
        deleted++;
        console.log(`  - ${change.path} (${removed} chunks removed)`);
      }
      continue;
    }

    const abs = resolve(targetPath, change.path);
    const result = await reindexOneFile(config, targetPath, projectId, importResolver, abs);
    switch (result.status) {
      case 'indexed':
        reindexed++;
        console.log(`  ~ ${result.relPath} (${result.chunks} chunks${result.staleRemoved ? `, ${result.staleRemoved} stale removed` : ''})`);
        break;
      case 'cleared':
      case 'deleted':
        deleted++;
        console.log(`  - ${result.relPath} (${result.removed} chunks removed)`);
        break;
      case 'error':
        failed++;
        console.error(`  ! ${result.relPath}: ${result.error}`);
        break;
    }
  }

  // Update stored metadata with the new HEAD + timestamp
  await upsertProject(config, projectId, projectName, targetPath, {
    grammarVersion: grammarVersion || undefined,
    gitHead: currentHead ?? meta.git_head ?? undefined,
    lastIndexedAt: Date.now(),
  });
  await setDefaultProject(config, projectId, targetPath);

  console.log(`\nIncremental done. ${reindexed} reindexed, ${deleted} deleted${failed ? `, ${failed} failed` : ''}.`);
  if (isGit) {
    console.log(`Indexed at: ${currentHead?.slice(0, 12) ?? 'unknown'}`);
  }
  console.log(`Note: CORTEX_REPORT.md is now stale — run \`cortex index\` for a refreshed report.`);
  return true;
}

async function generateReport(
  targetPath: string,
  projectName: string,
  projectId: string,
  allChunkRows: ChunkRow[],
  resolvedRelationships: RelationshipRow[],
  gitHead: string | null,
  indexedAtEpochMs: number,
): Promise<void> {
  // Per-file chunk counts for god-file detection
  const fileChunkCounts = new Map<string, number>();
  const languageCounts = new Map<string, number>();
  const chunkTypeCounts = new Map<string, number>();
  const symbolNames: string[] = [];

  for (const chunk of allChunkRows) {
    fileChunkCounts.set(chunk.file_path, (fileChunkCounts.get(chunk.file_path) || 0) + 1);
    if (chunk.language) languageCounts.set(chunk.language, (languageCounts.get(chunk.language) || 0) + 1);
    if (chunk.chunk_type) chunkTypeCounts.set(chunk.chunk_type, (chunkTypeCounts.get(chunk.chunk_type) || 0) + 1);
    if (chunk.symbol_name) symbolNames.push(chunk.symbol_name);
  }

  // God files: top 5 by chunk concentration
  const sortedFiles = [...fileChunkCounts.entries()].sort((a, b) => b[1] - a[1]);
  const godFiles = sortedFiles.slice(0, 5);
  const totalFiles = sortedFiles.length;

  // Relationship type counts
  const relTypeCounts = new Map<string, number>();
  for (const rel of resolvedRelationships) {
    relTypeCounts.set(rel.rel_type, (relTypeCounts.get(rel.rel_type) || 0) + 1);
  }

  // Most-connected symbols (appear most in relationships as source or target)
  const symbolEdgeCount = new Map<string, number>();
  const chunkIdToSymbol = new Map<string, string>();
  for (const chunk of allChunkRows) {
    if (chunk.symbol_name) chunkIdToSymbol.set(chunk.chunk_id, chunk.symbol_name);
  }
  for (const rel of resolvedRelationships) {
    const srcSym = chunkIdToSymbol.get(rel.source_chunk_id);
    const tgtSym = chunkIdToSymbol.get(rel.target_chunk_id);
    if (srcSym) symbolEdgeCount.set(srcSym, (symbolEdgeCount.get(srcSym) || 0) + 1);
    if (tgtSym) symbolEdgeCount.set(tgtSym, (symbolEdgeCount.get(tgtSym) || 0) + 1);
  }
  const topSymbols = [...symbolEdgeCount.entries()].sort((a, b) => b[1] - a[1]).slice(0, 10);

  const indexedAtIso = new Date(indexedAtEpochMs).toISOString();
  const indexedAtLine = gitHead
    ? `Indexed at: \`${gitHead.slice(0, 12)}\` (${indexedAtIso})`
    : `Indexed at: ${indexedAtIso}`;
  const lines = [
    `# Cortex Report: ${projectName}`,
    ``,
    `> Auto-generated by \`cortex index\` on ${indexedAtIso}. This file becomes stale after incremental (\`--watch\` or \`--incremental\`) updates.`,
    ``,
    `${indexedAtLine}`,
    ``,
    `## Summary`,
    ``,
    `| Metric | Value |`,
    `|--------|-------|`,
    `| Files | ${totalFiles} |`,
    `| Chunks | ${allChunkRows.length} |`,
    `| Relationships | ${resolvedRelationships.length} |`,
    `| Languages | ${[...languageCounts.keys()].join(', ') || 'none'} |`,
    ``,
    `## Languages`,
    ``,
    ...[...languageCounts.entries()].sort((a, b) => b[1] - a[1]).map(([lang, count]) =>
      `- **${lang}**: ${count} chunks`
    ),
    ``,
    `## God Files (highest chunk concentration)`,
    ``,
    ...godFiles.map(([file, count]) => {
      const pct = ((count / allChunkRows.length) * 100).toFixed(1);
      return `- \`${file}\` — ${count} chunks (${pct}%)`;
    }),
    ``,
    `## Chunk Types`,
    ``,
    ...[...chunkTypeCounts.entries()].sort((a, b) => b[1] - a[1]).map(([type, count]) =>
      `- **${type}**: ${count}`
    ),
    ``,
    `## Relationship Types`,
    ``,
    ...(relTypeCounts.size > 0
      ? [...relTypeCounts.entries()].sort((a, b) => b[1] - a[1]).map(([type, count]) =>
          `- **${type}**: ${count}`)
      : ['- none']),
    ``,
    `## Most-Connected Symbols`,
    ``,
    ...(topSymbols.length > 0
      ? topSymbols.map(([sym, count]) => `- \`${sym}\` — ${count} edges`)
      : ['- none']),
    ``,
  ];

  const reportPath = resolve(targetPath, 'CORTEX_REPORT.md');
  await writeFile(reportPath, lines.join('\n'), 'utf-8');
  console.log(`Report: ${reportPath}`);
}

interface ReindexResult {
  status: 'indexed' | 'cleared' | 'deleted' | 'skipped' | 'error';
  relPath: string;
  chunks?: number;
  staleRemoved?: number;
  removed?: number;
  error?: string;
}

/**
 * Re-index a single file: upsert chunks, delete stale chunks for that file,
 * replace file-scoped relationships. Handles missing files (ENOENT → delete).
 */
async function reindexOneFile(
  config: CortexConfig,
  targetPath: string,
  projectId: string,
  importResolver: ImportResolver,
  filePath: string,
): Promise<ReindexResult> {
  const relPath = relative(targetPath, filePath).replace(/\\/g, '/');
  try {
    const content = await readFile(filePath, 'utf-8');
    if (content.length === 0 || content.length > 100_000) {
      const removed = await deleteFileChunks(config, projectId, relPath);
      return removed > 0
        ? { status: 'cleared', relPath, removed }
        : { status: 'skipped', relPath };
    }

    const chunks = await chunkFileAST(projectId, relPath, content);
    if (chunks.length === 0) {
      const removed = await deleteFileChunks(config, projectId, relPath);
      return removed > 0
        ? { status: 'cleared', relPath, removed }
        : { status: 'skipped', relPath };
    }

    const embeddings = await embedBatch(chunks.map(c => contextPrefix(c) + '\n' + c.content));

    const chunkRows: ChunkRow[] = [];
    const pendingRels: PendingRelationship[] = [];
    const symbolIndex = new Map<string, Set<string>>();
    const fileIndex = new Map<string, string>();

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
        chunk_source: c.chunk_source,
      });

      for (const called of c.calls) {
        pendingRels.push({ source_chunk_id: c.chunk_id, source_file_path: c.file_path, target_ref: called, rel_type: 'calls' });
      }
      for (const imp of c.imports) {
        pendingRels.push({ source_chunk_id: c.chunk_id, source_file_path: c.file_path, target_ref: imp, rel_type: 'imports' });
      }
      for (const exp of c.exports) {
        pendingRels.push({ source_chunk_id: c.chunk_id, source_file_path: c.file_path, target_ref: exp, rel_type: 'exports' });
      }
    }

    await upsertChunks(config, chunkRows);
    const currentIds = chunkRows.map(c => c.chunk_id);
    const staleDeleted = await deleteStaleFileChunks(config, projectId, relPath, currentIds);
    const resolvedRels = resolveRelationships(pendingRels, symbolIndex, fileIndex, importResolver);
    await replaceFileRelationships(config, projectId, relPath, resolvedRels);
    return { status: 'indexed', relPath, chunks: chunkRows.length, staleRemoved: staleDeleted };
  } catch (err: any) {
    if (err.code === 'ENOENT') {
      const removed = await deleteFileChunks(config, projectId, relPath);
      return { status: 'deleted', relPath, removed };
    }
    if (err.code === 'EISDIR') {
      return { status: 'skipped', relPath };
    }
    return { status: 'error', relPath, error: err.message };
  }
}

async function watchAndReindex(
  config: CortexConfig,
  targetPath: string,
  projectId: string,
  projectName: string,
  importResolver: ImportResolver
): Promise<void> {
  const DEBOUNCE_MS = 500;
  const pending = new Map<string, NodeJS.Timeout>();

  async function reindexFile(filePath: string): Promise<void> {
    const result = await reindexOneFile(config, targetPath, projectId, importResolver, filePath);
    switch (result.status) {
      case 'indexed':
        console.log(`  [watch] Reindexed ${result.relPath} (${result.chunks} chunks${result.staleRemoved ? `, ${result.staleRemoved} stale removed` : ''})`);
        break;
      case 'cleared':
        console.log(`  [watch] Cleared ${result.relPath} (${result.removed} chunks removed)`);
        break;
      case 'deleted':
        console.log(`  [watch] Deleted ${result.relPath} (${result.removed} chunks removed)`);
        break;
      case 'error':
        console.error(`  [watch] Error ${result.relPath}: ${result.error}`);
        break;
    }
  }

  console.log(`\nWatching ${targetPath} for changes... (Ctrl+C to stop)`);

  const ac = new AbortController();
  process.on('SIGINT', () => { ac.abort(); process.exit(0); });

  try {
    const watcher = watch(targetPath, { recursive: true, signal: ac.signal });
    for await (const event of watcher) {
      if (!event.filename) continue;
      const fullPath = resolve(targetPath, event.filename);
      const ext = fullPath.substring(fullPath.lastIndexOf('.')).toLowerCase();

      // Skip non-code files, ignored filenames, and ignored directories
      if (!CODE_EXTS.has(ext)) continue;
      if (IGNORE_FILES.has(basename(event.filename))) continue;
      if (event.filename.split('/').some(part => IGNORE_DIRS.has(part) || part.startsWith('.'))) continue;

      // Debounce: clear previous timer for this file, set new one
      const existing = pending.get(fullPath);
      if (existing) clearTimeout(existing);
      pending.set(fullPath, setTimeout(() => {
        pending.delete(fullPath);
        reindexFile(fullPath).catch(err => console.error(`  [watch] ${err.message}`));
      }, DEBOUNCE_MS));
    }
  } catch (err: any) {
    if (err.name !== 'AbortError') throw err;
  }
}

// --- search ---
async function cmdSearch() {
  const config = await loadConfig();
  requireConfig(config);
  await ensureSchema(config);

  const rawQuery = args[1];
  if (!rawQuery) { console.error('Usage: cortex search "query"'); process.exit(1); }

  // Parse out kind:/lang:/name:/file: filter tokens before embedding/FTS.
  const { textQuery, filters } = parseQueryFilters(rawQuery);
  const filtersActive = hasFilters(filters);

  const projectId = await getProjectId(config);
  await emitStalenessHint(config, projectId);
  if (filtersActive) {
    await emitGrammarVersionWarning(config, projectId, filters);
  }

  const topK = parseInt(getFlag('--top-k') || '15');
  const offset = parseInt(getFlag('--offset') || '0');

  // If filters were used but no free text remained, we still need *some*
  // signal for vector/FTS. Fall back to the raw query in that edge case so
  // the embedder gets a deterministic input.
  const queryForRanking = textQuery.length > 0 ? textQuery : rawQuery;

  if (hasFlag('--keyword')) {
    // FTS-only mode. Sanitize the free-form query first so punctuation or
    // stripped filter tokens do not trigger an FTS5 parse error.
    const ftsQuery = sanitizeFtsQuery(queryForRanking);
    const result = ftsQuery
      ? await keywordSearch(config, ftsQuery, projectId, topK, offset, filters)
      : { chunks: [], total: 0 };
    output(result);
  } else if (hasFlag('--vector')) {
    // Vector-only mode (legacy default)
    console.error('Loading model...');
    const vector = await embed(queryForRanking);
    const result = await vectorSearch(config, vector, projectId, topK, offset, filters);
    output(result);
  } else {
    // Hybrid mode (new default): vector + FTS with RRF fusion
    const rrfK = parseInt(getFlag('--rrf-k') || '60');
    console.error('Loading model...');
    const vector = await embed(queryForRanking);
    const result = await hybridSearch(config, vector, queryForRanking, projectId, topK, rrfK, offset, filters);
    output(result);
  }
}

// --- Shared output schema for agent-first commands ---
interface ConfidenceMetadata {
  index_is_stale: boolean;
  index_staleness_reason: string | null;
  truncated: boolean;
  budget_tokens: number;
}

function estimateTokens(text: string): number {
  return Math.ceil(text.length / 4);
}

async function getStalenessInfo(config: CortexConfig, projectId: string): Promise<{ is_stale: boolean; reason: string | null }> {
  const meta = await getProjectMeta(config, projectId);
  if (!meta) return { is_stale: false, reason: null };

  const projectPath = meta.path ?? resolve('.');
  const currentHead = getGitHead(projectPath);
  if (!currentHead) return { is_stale: false, reason: null };

  if (meta.git_head && meta.git_head !== currentHead) {
    const behind = countCommitsBehind(projectPath, meta.git_head);
    const count = behind != null ? `${behind} commit${behind === 1 ? '' : 's'}` : 'commits';
    return { is_stale: true, reason: `index is ${count} behind HEAD` };
  }
  return { is_stale: false, reason: null };
}

// --- context ---
async function cmdContext() {
  const config = await loadConfig();
  requireConfig(config);
  await ensureSchema(config);

  const rawQuery = args[1];
  if (!rawQuery) { console.error('Usage: cortex context "symbol-or-query"'); process.exit(1); }

  // P5: extract filter tokens before symbol lookup / embedding.
  const { textQuery, filters } = parseQueryFilters(rawQuery);
  const filtersActive = hasFilters(filters);
  // Symbol lookup needs a clean name; fall back to raw if filters consumed everything.
  const query = textQuery.length > 0 ? textQuery : rawQuery;

  const projectId = await getProjectId(config);
  const BUDGET_TOKENS = 2000;

  // Staleness check
  const staleness = await getStalenessInfo(config, projectId);
  if (filtersActive) {
    await emitGrammarVersionWarning(config, projectId, filters);
  }

  // Try exact symbol match first, then fall back to hybrid search.
  // When filters are active, skip the symbol short-circuit — the user is asking
  // for filtered results, not "the chunk literally named X".
  console.error('Loading model...');
  const [symbolMatches, vector] = await Promise.all([
    filtersActive ? Promise.resolve([] as any[]) : findChunksBySymbol(config, projectId, query),
    embed(query),
  ]);

  let primaryMatches: any[];
  if (symbolMatches.length > 0) {
    // Exact symbol match — use these as primary
    primaryMatches = symbolMatches.slice(0, 5).map((c, i) => ({
      chunk_id: c.chunk_id,
      file_path: c.file_path,
      symbol_name: c.symbol_name,
      chunk_type: c.chunk_type,
      start_line: c.start_line,
      end_line: c.end_line,
      content: c.content,
      language: c.language,
      match_type: 'symbol',
      rank: i + 1,
    }));
  } else {
    // Hybrid search fallback (with P5 filters threaded through)
    const searchResult = await hybridSearch(config, vector, query, projectId, 5, 60, 0, filters);
    primaryMatches = searchResult.chunks.map((c, i) => ({
      chunk_id: c.chunk_id,
      file_path: c.file_path,
      symbol_name: c.symbol_name,
      chunk_type: c.chunk_type,
      start_line: c.start_line,
      end_line: c.end_line,
      content: c.content,
      language: c.language,
      match_type: c.source,
      rank: i + 1,
    }));
  }

  if (primaryMatches.length === 0) {
    output({
      primary_matches: [],
      neighbor_chunks: [],
      key_files: [],
      confidence_notes: [],
      suggested_next_queries: [],
      metadata: {
        index_is_stale: staleness.is_stale,
        index_staleness_reason: staleness.reason,
        truncated: false,
        budget_tokens: BUDGET_TOKENS,
      } satisfies ConfidenceMetadata,
    });
    return;
  }

  // Get depth-1 neighbors
  const primaryIds = primaryMatches.map((m: any) => m.chunk_id);
  const neighbors = await getDirectNeighbors(config, primaryIds, 15);

  // Budget tracking: start trimming if output gets too large
  let usedTokens = 0;
  for (const m of primaryMatches) {
    usedTokens += estimateTokens(m.content ?? '') + 30; // 30 for metadata fields
  }

  // Trim neighbors by effective confidence score (highest first). Legacy edges
  // from pre-P4 indexes have null scores; map them to tier defaults so they
  // sort comparably with freshly-scored edges. This matches the SQL ordering
  // in getDirectNeighbors / getTransitiveDependents / traverseRelationships.
  const tierDefault: Record<string, number> = { EXTRACTED: 0.9, INFERRED: 0.55, AMBIGUOUS: 0.35 };
  const effectiveScore = (e: any): number =>
    e.confidence_score != null ? e.confidence_score : (tierDefault[e.confidence] ?? 0.35);
  const sortedEdges = [...neighbors.edges].sort((a, b) => effectiveScore(b) - effectiveScore(a));

  // Only keep neighbor nodes that are referenced by surviving edges
  const neighborNodeMap = new Map(neighbors.nodes.map((n: any) => [n.chunk_id, n]));
  const keptNeighborIds = new Set<string>();
  const keptEdges: any[] = [];
  let truncated = false;

  for (const edge of sortedEdges) {
    const neighborId = primaryIds.includes(edge.source) ? edge.target : edge.source;
    const node = neighborNodeMap.get(neighborId);
    const edgeTokenCost = 20 + (node ? 15 : 0);

    if (usedTokens + edgeTokenCost > BUDGET_TOKENS) {
      truncated = true;
      break;
    }

    keptEdges.push(edge);
    keptNeighborIds.add(neighborId);
    usedTokens += edgeTokenCost;
  }

  const neighborChunks = [...keptNeighborIds]
    .map(id => neighborNodeMap.get(id))
    .filter(Boolean)
    .map((n: any) => ({
      chunk_id: n.chunk_id,
      file_path: n.file_path,
      symbol_name: n.symbol_name,
      chunk_type: n.chunk_type,
      start_line: n.start_line,
      end_line: n.end_line,
      relationship: keptEdges
        .filter(e => e.source === n.chunk_id || e.target === n.chunk_id)
        .map(e => ({ rel_type: e.rel_type, confidence: e.confidence, confidence_score: e.confidence_score ?? null, direction: e.target === n.chunk_id ? 'outgoing' : 'incoming' })),
    }));

  // Key files: unique files from primary + neighbor matches
  const fileSet = new Set<string>();
  for (const m of primaryMatches) fileSet.add(m.file_path);
  for (const n of neighborChunks) fileSet.add(n.file_path);
  const keyFiles = [...fileSet].slice(0, 8);

  // Confidence notes
  const confidenceNotes: string[] = [];
  const ambiguousEdges = keptEdges.filter(e => e.confidence === 'AMBIGUOUS');
  if (ambiguousEdges.length > 0) {
    confidenceNotes.push(`${ambiguousEdges.length} neighbor edge(s) are AMBIGUOUS — may be name collisions`);
  }
  if (primaryMatches[0]?.match_type !== 'symbol') {
    confidenceNotes.push('no exact symbol match — results are from hybrid search');
  }
  if (staleness.is_stale) {
    confidenceNotes.push(`index is stale: ${staleness.reason}`);
  }

  // Suggested next queries
  const suggestedNextQueries: string[] = [];
  if (truncated) {
    suggestedNextQueries.push(`cortex impact --symbol "${query}" (for full blast radius)`);
  }
  if (primaryMatches.length > 0) {
    const topFile = primaryMatches[0].file_path;
    suggestedNextQueries.push(`Read ${topFile}:${primaryMatches[0].start_line} for full source`);
  }

  // Strip content from primary matches to save tokens (agent can read the file if needed)
  const compactPrimary = primaryMatches.map((m: any) => ({
    chunk_id: m.chunk_id,
    file_path: m.file_path,
    symbol_name: m.symbol_name,
    chunk_type: m.chunk_type,
    start_line: m.start_line,
    end_line: m.end_line,
    language: m.language,
    match_type: m.match_type,
    rank: m.rank,
    content_preview: (m.content ?? '').slice(0, 300),
  }));

  output({
    primary_matches: compactPrimary,
    neighbor_chunks: neighborChunks,
    key_files: keyFiles,
    confidence_notes: confidenceNotes,
    suggested_next_queries: suggestedNextQueries,
    metadata: {
      index_is_stale: staleness.is_stale,
      index_staleness_reason: staleness.reason,
      truncated,
      budget_tokens: BUDGET_TOKENS,
    } satisfies ConfidenceMetadata,
  });
}

// --- impact ---
async function cmdImpact() {
  const config = await loadConfig();
  requireConfig(config);
  await ensureSchema(config);

  const projectId = await getProjectId(config);
  const staleness = await getStalenessInfo(config, projectId);

  if (hasFlag('--from-diff')) {
    await cmdImpactFromDiff(config, projectId, staleness);
  } else if (hasFlag('--symbol')) {
    const symbol = getFlag('--symbol');
    if (!symbol) { console.error('Usage: cortex impact --symbol "name"'); process.exit(1); }
    await cmdImpactSymbol(config, projectId, symbol, staleness);
  } else {
    // Default: treat first positional arg as symbol
    const symbol = args[1];
    if (!symbol) {
      console.error('Usage: cortex impact --symbol "name" | cortex impact --from-diff [sha]');
      process.exit(1);
    }
    await cmdImpactSymbol(config, projectId, symbol, staleness);
  }
}

async function cmdImpactSymbol(
  config: CortexConfig,
  projectId: string,
  symbol: string,
  staleness: { is_stale: boolean; reason: string | null },
) {
  // Find seed chunks by symbol name
  const seeds = await findChunksBySymbol(config, projectId, symbol);
  if (seeds.length === 0) {
    // Fall back to hybrid search
    console.error('Loading model...');
    const vector = await embed(symbol);
    const searchResult = await hybridSearch(config, vector, symbol, projectId, 3);
    if (searchResult.chunks.length === 0) {
      output({
        symbol,
        mode: 'symbol',
        affected_files: [],
        affected_symbols: [],
        edges: [],
        confidence_notes: ['no matching symbol or chunk found'],
        metadata: {
          index_is_stale: staleness.is_stale,
          index_staleness_reason: staleness.reason,
          truncated: false,
          budget_tokens: 0,
        } satisfies ConfidenceMetadata,
      });
      return;
    }
    // Use search results as seeds
    const seedIds = searchResult.chunks.map(c => c.chunk_id);
    const impact = await getTransitiveDependents(config, projectId, seedIds, 3, 30);
    outputImpactResult(symbol, 'symbol', impact, staleness, ['seed resolved via search, not exact symbol match']);
    return;
  }

  const seedIds = seeds.map(s => s.chunk_id);
  const impact = await getTransitiveDependents(config, projectId, seedIds, 3, 30);
  outputImpactResult(symbol, 'symbol', impact, staleness, []);
}

async function cmdImpactFromDiff(
  config: CortexConfig,
  projectId: string,
  staleness: { is_stale: boolean; reason: string | null },
) {
  const meta = await getProjectMeta(config, projectId);
  const projectPath = meta?.path ?? resolve('.');

  // Determine base SHA: explicit arg after --from-diff, or stored git_head
  const flagIdx = args.indexOf('--from-diff');
  const explicitSha = flagIdx >= 0 && flagIdx + 1 < args.length && !args[flagIdx + 1].startsWith('--')
    ? args[flagIdx + 1]
    : null;
  const baseSha = explicitSha ?? meta?.git_head ?? null;

  if (!baseSha) {
    console.error('No base SHA available. Provide one: cortex impact --from-diff <sha>');
    process.exit(1);
  }

  // Get changed files from git diff
  const changes = gitDiffAgainstWorkingTree(projectPath, baseSha);
  if (!changes || changes.length === 0) {
    output({
      mode: 'from-diff',
      base_sha: baseSha,
      changed_files: [],
      affected_files: [],
      affected_symbols: [],
      edges: [],
      confidence_notes: ['no changes detected against base SHA'],
      metadata: {
        index_is_stale: staleness.is_stale,
        index_staleness_reason: staleness.reason,
        truncated: false,
        budget_tokens: 0,
      } satisfies ConfidenceMetadata,
    });
    return;
  }

  // Filter to indexable files
  const indexableChanges = changes.filter(c => {
    const e = c.path.substring(c.path.lastIndexOf('.')).toLowerCase();
    if (!CODE_EXTS.has(e)) return false;
    if (IGNORE_FILES.has(basename(c.path))) return false;
    if (c.path.split('/').some(p => IGNORE_DIRS.has(p) || p.startsWith('.'))) return false;
    return true;
  });

  // Find all chunks in changed files, then compute transitive dependents.
  //
  // The DB reflects the indexed base, not the working tree, so we must look up
  // chunks under the path they had at index time:
  //   - A/M: chunks live at change.path (if already indexed; new files may be empty).
  //   - D:   chunks still live at change.path — this is exactly the "what depends on
  //          the thing about to disappear?" case that impact analysis exists for.
  //   - R:   chunks still live at change.oldPath; the new path is not yet indexed.
  const seedChunkIds: string[] = [];
  for (const change of indexableChanges) {
    const lookupPath = change.status === 'R' ? change.oldPath : change.path;
    if (!lookupPath) continue;
    const fileChunks = await findChunksByFile(config, projectId, lookupPath);
    for (const c of fileChunks) seedChunkIds.push(c.chunk_id);
  }

  const impact = seedChunkIds.length > 0
    ? await getTransitiveDependents(config, projectId, seedChunkIds, 3, 30)
    : { nodes: [], edges: [], depth_reached: 0 };

  // Summarize changed files
  const changedFiles = indexableChanges.map(c => ({
    path: c.path,
    status: c.status,
    old_path: c.oldPath ?? null,
  }));

  // Collect affected files and symbols from impact graph
  const affectedFiles = new Set<string>();
  const affectedSymbols: { symbol: string; file: string; chunk_type: string | null }[] = [];
  for (const node of impact.nodes) {
    affectedFiles.add(node.file_path);
    if (node.symbol_name) {
      affectedSymbols.push({
        symbol: node.symbol_name,
        file: node.file_path,
        chunk_type: node.chunk_type,
      });
    }
  }

  // Deduplicate affected symbols
  const seenSymbols = new Set<string>();
  const uniqueSymbols = affectedSymbols.filter(s => {
    const key = `${s.file}:${s.symbol}`;
    if (seenSymbols.has(key)) return false;
    seenSymbols.add(key);
    return true;
  });

  const confidenceNotes: string[] = [];
  const ambiguousEdges = impact.edges.filter(e => e.confidence === 'AMBIGUOUS');
  if (ambiguousEdges.length > 0) {
    confidenceNotes.push(`${ambiguousEdges.length} edge(s) are AMBIGUOUS — impact may be overstated`);
  }
  if (staleness.is_stale) {
    confidenceNotes.push(`index is stale: ${staleness.reason}`);
  }

  output({
    mode: 'from-diff',
    base_sha: baseSha,
    changed_files: changedFiles,
    affected_files: [...affectedFiles].sort(),
    affected_symbols: uniqueSymbols.slice(0, 30),
    edges: impact.edges.slice(0, 50),
    depth_reached: impact.depth_reached,
    confidence_notes: confidenceNotes,
    metadata: {
      index_is_stale: staleness.is_stale,
      index_staleness_reason: staleness.reason,
      truncated: impact.edges.length > 50,
      budget_tokens: 0,
    } satisfies ConfidenceMetadata,
  });
}

function outputImpactResult(
  label: string,
  mode: string,
  impact: { nodes: any[]; edges: any[]; depth_reached: number },
  staleness: { is_stale: boolean; reason: string | null },
  extraNotes: string[],
) {
  const affectedFiles = new Set<string>();
  const affectedSymbols: { symbol: string; file: string; chunk_type: string | null }[] = [];

  for (const node of impact.nodes) {
    affectedFiles.add(node.file_path);
    if (node.symbol_name) {
      affectedSymbols.push({
        symbol: node.symbol_name,
        file: node.file_path,
        chunk_type: node.chunk_type,
      });
    }
  }

  const seenSymbols = new Set<string>();
  const uniqueSymbols = affectedSymbols.filter(s => {
    const key = `${s.file}:${s.symbol}`;
    if (seenSymbols.has(key)) return false;
    seenSymbols.add(key);
    return true;
  });

  const confidenceNotes = [...extraNotes];
  const ambiguousEdges = impact.edges.filter(e => e.confidence === 'AMBIGUOUS');
  if (ambiguousEdges.length > 0) {
    confidenceNotes.push(`${ambiguousEdges.length} edge(s) are AMBIGUOUS — impact may be overstated`);
  }
  if (staleness.is_stale) {
    confidenceNotes.push(`index is stale: ${staleness.reason}`);
  }

  output({
    symbol: label,
    mode,
    affected_files: [...affectedFiles].sort(),
    affected_symbols: uniqueSymbols.slice(0, 30),
    edges: impact.edges.slice(0, 50),
    depth_reached: impact.depth_reached,
    confidence_notes: confidenceNotes,
    metadata: {
      index_is_stale: staleness.is_stale,
      index_staleness_reason: staleness.reason,
      truncated: impact.edges.length > 50,
      budget_tokens: 0,
    } satisfies ConfidenceMetadata,
  });
}

// --- relationships ---
async function cmdRelationships() {
  const config = await loadConfig();
  requireConfig(config);
  await ensureSchema(config);

  const symbol = args[1];
  if (!symbol) { console.error('Usage: cortex relationships "symbol"'); process.exit(1); }

  const projectId = await getProjectId(config);
  const depth = parseInt(getFlag('--depth') || '2');
  const verbose = hasFlag('--verbose');
  const result = await traverseRelationships(config, projectId, symbol, depth);

  // Strip reasoning unless --verbose
  if (!verbose) {
    result.edges = result.edges.map((e: any) => {
      const { confidence_reasoning, ...rest } = e;
      return rest;
    });
  }

  output(result);
}

// --- modules ---
async function cmdModules() {
  const config = await loadConfig();
  requireConfig(config);
  await ensureSchema(config);

  const projectId = await getProjectId(config);
  const minSize = parseInt(getFlag('--min-size') ?? '1');

  await emitStalenessHint(config, projectId);

  console.error('Loading project graph...');
  const graphData = await getProjectGraphData(config, projectId);

  if (graphData.chunks.length === 0) {
    console.error('No chunks found. Run: cortex index');
    process.exit(1);
  }

  const result = clusterProject(graphData.chunks, graphData.relationships);

  if (minSize > 1) {
    result.modules = result.modules.filter(m => m.file_count >= minSize);
    result.module_count = result.modules.length;
  }

  output({ project_id: projectId, ...result });
}

// --- status ---
async function cmdStatus() {
  const config = await loadConfig();
  requireConfig(config);
  await ensureSchema(config);

  const projectId = await getProjectId(config);
  await emitStalenessHint(config, projectId);
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

// --- grammars ---
async function cmdGrammars() {
  const sub = args[1];

  if (sub === 'install') {
    const sourcePath = args[2];
    if (!sourcePath) {
      console.error('Usage: cortex grammars install <path-to-wasm-directory>');
      process.exit(1);
    }
    const installed = await installFromLocal(resolve(sourcePath));
    if (installed.length > 0) {
      console.log(`Installed grammars: ${installed.join(', ')}`);
    } else {
      console.log('No matching grammar files found in the specified path.');
    }
    return;
  }

  // Default: show status
  const status = await grammarStatus();
  const version = await computeGrammarVersionHash();
  console.log(`Grammars directory: ${grammarsDir()}`);
  console.log(`Grammar version hash: ${version || '(none)'}\n`);
  for (const g of status) {
    console.log(`  ${g.available ? `[ok]` : '[--]'} ${g.name}${g.available ? ` (${g.source})` : ''}`);
  }
  if (status.some(g => !g.available)) {
    console.log('\nMissing grammars will be auto-downloaded on next `cortex index`.');
    console.log('For offline install: cortex grammars install <path-to-wasm-directory>');
  }
}

// --- git helpers ---

/** Return current git HEAD sha, or null if not a git repo / git unavailable. */
function getGitHead(cwd: string): string | null {
  try {
    const sha = execFileSync('git', ['rev-parse', 'HEAD'], {
      cwd,
      stdio: ['ignore', 'pipe', 'ignore'],
      encoding: 'utf-8',
    }).trim();
    return sha || null;
  } catch {
    return null;
  }
}

/** Count commits between old..HEAD. Returns null if git diff fails. */
function countCommitsBehind(cwd: string, oldSha: string): number | null {
  try {
    const out = execFileSync('git', ['rev-list', '--count', `${oldSha}..HEAD`], {
      cwd,
      stdio: ['ignore', 'pipe', 'ignore'],
      encoding: 'utf-8',
    }).trim();
    const n = parseInt(out, 10);
    return Number.isFinite(n) ? n : null;
  } catch {
    return null;
  }
}

export interface FileChange {
  status: 'A' | 'M' | 'D' | 'R';
  path: string;
  /** Set when status === 'R' — previous path before rename */
  oldPath?: string;
}

/**
 * Diff a stored sha against the current working tree, including untracked files.
 * Returns null if git diff fails (e.g. stored sha no longer exists after history rewrite).
 */
function gitDiffAgainstWorkingTree(cwd: string, oldSha: string): FileChange[] | null {
  try {
    const out = execFileSync('git', ['diff', '--name-status', '-M', '-z', oldSha], {
      cwd,
      stdio: ['ignore', 'pipe', 'ignore'],
      encoding: 'utf-8',
    });
    const changes = parseNameStatusZ(out);

    // Supplement with untracked files (not yet git-added) so new files
    // are picked up by --incremental without requiring `git add` first.
    try {
      const untracked = execFileSync('git', ['ls-files', '--others', '--exclude-standard', '-z'], {
        cwd,
        stdio: ['ignore', 'pipe', 'ignore'],
        encoding: 'utf-8',
      });
      const alreadySeen = new Set(changes.map(c => c.path));
      for (const path of untracked.split('\0')) {
        if (path && !alreadySeen.has(path)) {
          changes.push({ status: 'A', path });
        }
      }
    } catch {
      // If ls-files fails, proceed with tracked diff only
    }

    return changes;
  } catch {
    return null;
  }
}

/**
 * Parse `git diff --name-status -z` output. The -z flag produces NUL-separated records:
 *   <status>\0<path>\0                         for A/M/D
 *   R<score>\0<old_path>\0<new_path>\0         for renames
 */
function parseNameStatusZ(output: string): FileChange[] {
  const changes: FileChange[] = [];
  const tokens = output.split('\0');
  let i = 0;
  while (i < tokens.length) {
    const tok = tokens[i];
    if (!tok) { i++; continue; }
    const status = tok.charAt(0);
    if (status === 'R' || status === 'C') {
      const oldPath = tokens[i + 1];
      const newPath = tokens[i + 2];
      if (oldPath && newPath) {
        changes.push({ status: 'R', path: newPath, oldPath });
      }
      i += 3;
    } else if (status === 'A' || status === 'M' || status === 'D') {
      const path = tokens[i + 1];
      if (path) changes.push({ status: status as 'A' | 'M' | 'D', path });
      i += 2;
    } else {
      i++;
    }
  }
  return changes;
}

/** Resolve an incremental plan using git when possible, mtime fallback otherwise. */
async function computeIncrementalChanges(
  targetPath: string,
  storedSha: string | null,
  storedIndexedAt: number | null,
  allFiles: string[],
  indexedFilePaths?: Set<string>,
): Promise<{ changes: FileChange[]; method: 'git' | 'mtime' | 'full' }> {
  // Prefer git when we have a stored sha
  if (storedSha) {
    const changes = gitDiffAgainstWorkingTree(targetPath, storedSha);
    if (changes !== null) {
      // Filter to indexable files only
      const filtered = changes.filter(c => {
        const checkPath = c.status === 'R' ? c.path : c.path;
        const e = checkPath.substring(checkPath.lastIndexOf('.')).toLowerCase();
        if (!CODE_EXTS.has(e)) return false;
        if (IGNORE_FILES.has(basename(checkPath))) return false;
        if (checkPath.split('/').some(p => IGNORE_DIRS.has(p) || p.startsWith('.'))) return false;
        return true;
      });
      return { changes: filtered, method: 'git' };
    }
  }

  // mtime fallback: scan the files we already collected for mtimes newer than last_indexed_at
  // Also detect deletions by comparing indexed file paths against current files on disk.
  if (storedIndexedAt) {
    const changes: FileChange[] = [];
    const currentRelPaths = new Set<string>();
    for (const abs of allFiles) {
      const relPath = relative(targetPath, abs).replace(/\\/g, '/');
      currentRelPaths.add(relPath);
      try {
        const st = await stat(abs);
        if (st.mtimeMs > storedIndexedAt) {
          changes.push({ status: 'M', path: relPath });
        }
      } catch {
        // Skip unreadable files
      }
    }
    // Detect deletions: files in the index but no longer on disk
    if (indexedFilePaths) {
      for (const indexed of indexedFilePaths) {
        if (!currentRelPaths.has(indexed)) {
          changes.push({ status: 'D', path: indexed });
        }
      }
    }
    return { changes, method: 'mtime' };
  }

  // No stored state at all — caller should do a full reindex
  return { changes: [], method: 'full' };
}

/**
 * Emit a one-line staleness hint to stderr when the index is behind HEAD.
 * Suppressed by --quiet flag or CORTEX_QUIET=1 env var.
 */
async function emitStalenessHint(config: CortexConfig, projectId: string): Promise<void> {
  if (hasFlag('--quiet')) return;
  if (process.env.CORTEX_QUIET === '1') return;

  const meta = await getProjectMeta(config, projectId);
  if (!meta) return;

  // Use the project's stored path (not cwd) so we compare against the right git repo.
  const projectPath = meta.path ?? resolve('.');
  const currentHead = getGitHead(projectPath);
  if (!currentHead) return; // Not a git repo — nothing to compare

  if (meta.git_head && meta.git_head !== currentHead) {
    const behind = countCommitsBehind(projectPath, meta.git_head);
    const count = behind != null ? `${behind} commit${behind === 1 ? '' : 's'}` : 'commits';
    process.stderr.write(`[cortex] index is ${count} behind HEAD, run: cortex index --incremental\n`);
  }
}

/**
 * P5: When `kind:` or `lang:` filters are in play, the result quality depends on
 * the AST chunker output stored in the index. If the indexed grammar version
 * differs from the current grammar bundle, surface a one-line warning so the
 * agent knows results may be inaccurate. Suppressed by --quiet / CORTEX_QUIET=1.
 */
async function emitGrammarVersionWarning(
  config: CortexConfig,
  projectId: string,
  filters: SearchFilters,
): Promise<void> {
  if (hasFlag('--quiet')) return;
  if (process.env.CORTEX_QUIET === '1') return;
  // Only kind: and lang: depend on the AST chunker output.
  if (!filters.kind && !filters.language) return;

  const meta = await getProjectMeta(config, projectId);
  if (!meta || !meta.grammar_version) return;

  const currentVersion = await computeGrammarVersionHash().catch(() => null);
  if (!currentVersion) return;

  if (meta.grammar_version !== currentVersion) {
    process.stderr.write(
      `[cortex] grammar version drift detected (indexed=${meta.grammar_version.slice(0, 7)}, current=${currentVersion.slice(0, 7)}); kind:/lang: filters may be inaccurate. Run: cortex index\n`,
    );
  }
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
        if (IGNORE_FILES.has(entry.name)) continue;
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

/**
 * Compute confidence score for a relationship edge.
 *
 * score = resolution_quality × source_multiplier
 *
 * Resolution quality:
 *   - File-index resolution (imports): 1.0
 *   - Single symbol-name match: 0.7
 *   - Multi-candidate (N matches): 1/N
 *
 * Source multiplier:
 *   - EXTRACTED (imports/exports from AST): ×1.0
 *   - INFERRED (single name match for calls): ×0.8
 *   - AMBIGUOUS (multi-target or unresolved): ×0.5
 */
function scoreRelationship(
  confidence: RelationshipConfidence,
  candidateCount: number,
  viaFileIndex: boolean,
): { score: number; reasoning: string } {
  // Resolution quality
  let quality: number;
  let qualityLabel: string;
  if (viaFileIndex && candidateCount >= 1) {
    quality = 1.0;
    qualityLabel = 'file-index resolution';
  } else if (candidateCount === 1) {
    quality = 0.7;
    qualityLabel = 'single symbol-name match';
  } else if (candidateCount > 1) {
    quality = 1 / candidateCount;
    qualityLabel = `${candidateCount}-way name collision`;
  } else {
    quality = 0.2;
    qualityLabel = 'unresolved';
  }

  // Source multiplier
  const multipliers: Record<string, number> = { EXTRACTED: 1.0, INFERRED: 0.8, AMBIGUOUS: 0.5 };
  const mult = multipliers[confidence] ?? 0.5;

  const score = Math.max(0, Math.min(1, quality * mult));
  const reasoning = `${qualityLabel} (${quality.toFixed(2)}) × ${confidence} (${mult.toFixed(1)}) = ${score.toFixed(2)}`;

  return { score, reasoning };
}

function resolveRelationships(
  pendingRelationships: PendingRelationship[],
  symbolIndex: Map<string, Set<string>>,
  fileIndex: Map<string, string>,
  importResolver: ImportResolver
): RelationshipRow[] {
  const deduped = new Map<string, RelationshipRow>();

  for (const rel of pendingRelationships) {
    let targetIds: string[];
    let confidence: RelationshipConfidence;
    let viaFileIndex = false;

    if (rel.rel_type === 'imports') {
      targetIds = resolveImportTargets(rel.target_ref, rel.source_file_path, fileIndex, importResolver);
      confidence = targetIds.length > 0 ? 'EXTRACTED' : 'AMBIGUOUS';
      viaFileIndex = targetIds.length > 0;
    } else if (rel.rel_type === 'exports') {
      targetIds = [...(symbolIndex.get(rel.target_ref) ?? [])];
      confidence = 'EXTRACTED';
    } else {
      // 'calls' — symbol name lookup; ambiguous if multiple targets (name collision)
      targetIds = [...(symbolIndex.get(rel.target_ref) ?? [])];
      confidence = targetIds.length === 1 ? 'INFERRED' : targetIds.length > 1 ? 'AMBIGUOUS' : 'AMBIGUOUS';
    }

    const { score, reasoning } = scoreRelationship(confidence, targetIds.length, viaFileIndex);

    for (const targetChunkId of targetIds) {
      const key = `${rel.source_chunk_id}:${targetChunkId}:${rel.rel_type}`;
      deduped.set(key, {
        source_chunk_id: rel.source_chunk_id,
        target_chunk_id: targetChunkId,
        rel_type: rel.rel_type,
        confidence,
        confidence_score: score,
        confidence_reasoning: reasoning,
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
