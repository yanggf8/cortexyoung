import { createHash } from 'crypto';

export type ChunkType = 'function' | 'class' | 'method' | 'documentation' | 'config';
export type ChunkSource = 'ast' | 'regex';

export interface Chunk {
  chunk_id: string;
  project_id: string;
  file_path: string;
  symbol_name?: string;
  chunk_type: ChunkType;
  start_line: number;
  end_line: number;
  content: string;
  content_hash: string;
  language: string;
  calls: string[];
  imports: string[];
  exports: string[];
  chunk_source?: ChunkSource;
}

const TOKEN_LIMIT = 400;
const MIN_TOKENS = 50;
const estimateTokens = (s: string) => Math.ceil(s.length / 4);

const CODE_EXTENSIONS = new Set(['.js', '.jsx', '.ts', '.tsx', '.py', '.go', '.rs', '.java', '.cpp', '.c', '.h']);

const LANG_MAP: Record<string, string> = {
  '.ts': 'typescript', '.tsx': 'typescript',
  '.js': 'javascript', '.jsx': 'javascript',
  '.py': 'python', '.go': 'go', '.rs': 'rust',
  '.java': 'java', '.cpp': 'cpp', '.c': 'c', '.h': 'c',
  '.md': 'markdown', '.json': 'json', '.yaml': 'yaml', '.yml': 'yaml',
};

function ext(filePath: string): string {
  return filePath.substring(filePath.lastIndexOf('.')).toLowerCase();
}

function hashContent(content: string): string {
  return createHash('sha256').update(content).digest('hex');
}

const CONTROL_FLOW = new Set(['if', 'else', 'for', 'while', 'do', 'switch', 'catch', 'return', 'throw', 'new', 'typeof', 'instanceof', 'delete', 'void']);

function extractCalls(content: string): string[] {
  const matches = content.match(/(\w+)\s*\(/g);
  if (!matches) return [];
  return [...new Set(matches.map(m => m.replace(/\s*\(/, '')).filter(m => !CONTROL_FLOW.has(m)))];
}

function extractImports(content: string): string[] {
  const imports: string[] = [];
  const jsImports = content.match(/import.*?from\s+['"]([^'"]+)['"]/g);
  if (jsImports) imports.push(...jsImports.map(i => i.match(/from\s+['"]([^'"]+)['"]/)![1]));
  const pyImports = content.match(/(?:import\s+(\w+)|from\s+(\w+)\s+import)/g);
  if (pyImports) imports.push(...pyImports.map(i => { const m = i.match(/(?:import\s+(\w+)|from\s+(\w+))/); return m ? (m[1] || m[2]) : ''; }).filter(Boolean));
  return imports;
}

function extractExports(content: string): string[] {
  const matches = content.match(/export\s+(?:function\s+(\w+)|const\s+(\w+)|class\s+(\w+))/g);
  if (!matches) return [];
  return matches.map(e => { const m = e.match(/export\s+(?:function\s+(\w+)|const\s+(\w+)|class\s+(\w+))/); return m ? (m[1] || m[2] || m[3]) : ''; }).filter(Boolean);
}

function makeChunk(projectId: string, filePath: string, content: string, startLine: number, endLine: number, chunkType: ChunkType, symbolName: string): Chunk {
  return {
    chunk_id: `${projectId}:${filePath}:${startLine}`,
    project_id: projectId,
    file_path: filePath,
    symbol_name: symbolName || undefined,
    chunk_type: chunkType,
    start_line: startLine,
    end_line: endLine,
    content: content.trim(),
    content_hash: hashContent(content),
    language: LANG_MAP[ext(filePath)] || 'unknown',
    calls: extractCalls(content),
    imports: extractImports(content),
    exports: extractExports(content),
  };
}

// --- Context prefix for embedding (not stored in DB) ---

export function contextPrefix(chunk: Chunk): string {
  const parts = chunk.file_path.split('/');
  const shortPath = parts.length > 2 ? parts.slice(-2).join('/') : chunk.file_path;
  const segments = [`file: ${shortPath}`];
  if (chunk.symbol_name) segments.push(`${chunk.chunk_type}: ${chunk.symbol_name}`);
  const prefix = `// ${segments.join(' | ')}`;
  return prefix.length > 80 ? prefix.slice(0, 80) : prefix;
}

// --- Post-processing: split oversized, merge undersized ---

/**
 * Create a derivative chunk that inherits metadata (chunk_source, calls, imports, exports)
 * from a parent. Used by split/merge so AST-extracted relationships survive post-processing.
 */
function deriveChunk(
  parent: Chunk,
  content: string,
  startLine: number,
  endLine: number,
  symbolName: string
): Chunk {
  return {
    chunk_id: `${parent.project_id}:${parent.file_path}:${startLine}`,
    project_id: parent.project_id,
    file_path: parent.file_path,
    symbol_name: symbolName || undefined,
    chunk_type: parent.chunk_type,
    start_line: startLine,
    end_line: endLine,
    content: content.trim(),
    content_hash: hashContent(content),
    language: parent.language,
    calls: parent.calls,
    imports: parent.imports,
    exports: parent.exports,
    chunk_source: parent.chunk_source,
  };
}

function splitOversized(chunks: Chunk[]): Chunk[] {
  const charLimit = TOKEN_LIMIT * 4;
  const result: Chunk[] = [];
  for (const chunk of chunks) {
    if (estimateTokens(chunk.content) <= TOKEN_LIMIT) {
      result.push(chunk);
      continue;
    }
    const lines = chunk.content.split('\n');
    // Single-line or unsplittable: hard truncate at char limit
    if (lines.length <= 1) {
      const truncated = chunk.content.slice(0, charLimit);
      result.push(deriveChunk(chunk, truncated, chunk.start_line, chunk.end_line, chunk.symbol_name || ''));
      continue;
    }
    const mid = Math.floor(lines.length / 2);
    // Find nearest blank line to midpoint
    let splitAt = mid;
    for (let offset = 0; offset <= mid; offset++) {
      if (mid + offset < lines.length && lines[mid + offset].trim() === '') { splitAt = mid + offset; break; }
      if (mid - offset >= 0 && lines[mid - offset].trim() === '') { splitAt = mid - offset; break; }
    }
    // Ensure we always make progress (never split at 0)
    if (splitAt <= 0) splitAt = mid;
    if (splitAt <= 0) splitAt = 1;
    const firstHalf = lines.slice(0, splitAt).join('\n');
    const secondHalf = lines.slice(splitAt).join('\n');
    const firstEnd = chunk.start_line + splitAt - 1;
    const secondStart = chunk.start_line + splitAt;
    const baseName = chunk.symbol_name || '';
    if (firstHalf.trim()) {
      result.push(deriveChunk(chunk, firstHalf, chunk.start_line, firstEnd, baseName ? `${baseName}:part_1` : ''));
    }
    if (secondHalf.trim()) {
      result.push(deriveChunk(chunk, secondHalf, secondStart, chunk.end_line, baseName ? `${baseName}:part_2` : ''));
    }
  }
  // Recurse if any chunk is still oversized
  if (result.some(c => estimateTokens(c.content) > TOKEN_LIMIT)) {
    return splitOversized(result);
  }
  return result;
}

function mergeUndersized(chunks: Chunk[]): Chunk[] {
  if (chunks.length < 2) return chunks;
  const result: Chunk[] = [chunks[0]];
  for (let i = 1; i < chunks.length; i++) {
    const prev = result[result.length - 1];
    const curr = chunks[i];
    const prevTokens = estimateTokens(prev.content);
    const currTokens = estimateTokens(curr.content);
    const canMerge =
      prevTokens < MIN_TOKENS &&
      prev.chunk_type === curr.chunk_type &&
      (!prev.symbol_name || !curr.symbol_name || prev.symbol_name === curr.symbol_name) &&
      prevTokens + currTokens <= TOKEN_LIMIT;
    if (canMerge) {
      const mergedContent = prev.content + '\n' + curr.content;
      const merged: Chunk = {
        chunk_id: prev.chunk_id,
        project_id: prev.project_id,
        file_path: prev.file_path,
        symbol_name: (curr.symbol_name || prev.symbol_name) || undefined,
        chunk_type: curr.chunk_type,
        start_line: prev.start_line,
        end_line: curr.end_line,
        content: mergedContent.trim(),
        content_hash: hashContent(mergedContent),
        language: prev.language,
        calls: [...new Set([...prev.calls, ...curr.calls])],
        imports: [...new Set([...prev.imports, ...curr.imports])],
        exports: [...new Set([...prev.exports, ...curr.exports])],
        chunk_source: prev.chunk_source ?? curr.chunk_source,
      };
      result[result.length - 1] = merged;
    } else {
      result.push(curr);
    }
  }
  return result;
}

function postProcess(chunks: Chunk[]): Chunk[] {
  return mergeUndersized(splitOversized(chunks));
}

// --- Import line detection ---

function isImportStart(line: string): boolean {
  const trimmed = line.trim();
  return /^import\s+/.test(trimmed) ||
    /^export\s+\{/.test(trimmed) ||
    /^export\s+type\s+\{/.test(trimmed) ||
    /^export\s+\*\s+from\s+/.test(trimmed);
}

function isPyImportStart(line: string): boolean {
  const trimmed = line.trim();
  return /^import\s+/.test(trimmed) || /^from\s+\w/.test(trimmed);
}

// --- JS/TS chunking by brace counting ---

function chunkJSTS(projectId: string, filePath: string, content: string): Chunk[] {
  const lines = content.split('\n');
  const chunks: Chunk[] = [];
  let current = '';
  let startLine = 1;
  let symbolName = '';
  let chunkType: ChunkType = 'function';
  let braceCount = 0;
  let inBlock = false;
  let importBuffer = '';
  let importStartLine = 0;
  let inImportBlock = false;

  function flushImports(beforeLine: number) {
    if (importBuffer.trim()) {
      chunks.push(makeChunk(projectId, filePath, importBuffer, importStartLine, beforeLine - 1, 'config', 'imports'));
      importBuffer = '';
    }
    inImportBlock = false;
  }

  for (let i = 0; i < lines.length; i++) {
    const line = lines[i];
    const trimmed = line.trim();

    // Multiline import continuation: buffer until line ends with a complete statement
    if (inImportBlock) {
      importBuffer += line + '\n';
      // Check if the import statement is complete (has closing quote or closing brace+from)
      if (/['"]\s*;?\s*$/.test(trimmed) || /}\s*from\s+['"].*['"]\s*;?\s*$/.test(trimmed) || trimmed.endsWith(';')) {
        inImportBlock = false;
      }
      continue;
    }

    if (!inBlock && isImportStart(trimmed)) {
      if (!importBuffer) importStartLine = i + 1;
      importBuffer += line + '\n';
      // Check if this import spans multiple lines (no closing quote/semicolon on this line)
      const isSingleLine = /['"]\s*;?\s*$/.test(trimmed) || trimmed.endsWith(';');
      if (!isSingleLine) {
        inImportBlock = true;
      }
      continue;
    }

    if (importBuffer && !isImportStart(trimmed)) {
      flushImports(i + 1);
    }

    if (isFunctionDecl(trimmed)) {
      if (current && inBlock) {
        chunks.push(makeChunk(projectId, filePath, current, startLine, i, chunkType, symbolName));
      }
      inBlock = true;
      chunkType = 'function';
      symbolName = extractFuncName(trimmed);
      startLine = i + 1;
      current = line + '\n';
      braceCount = (line.match(/{/g) || []).length - (line.match(/}/g) || []).length;
    } else if (isClassDecl(trimmed)) {
      if (current && inBlock) {
        chunks.push(makeChunk(projectId, filePath, current, startLine, i, chunkType, symbolName));
      }
      inBlock = true;
      chunkType = 'class';
      symbolName = (trimmed.match(/class\s+(\w+)/) || [])[1] || '';
      startLine = i + 1;
      current = line + '\n';
      braceCount = (line.match(/{/g) || []).length - (line.match(/}/g) || []).length;
    } else if (inBlock) {
      current += line + '\n';
      braceCount += (line.match(/{/g) || []).length - (line.match(/}/g) || []).length;
      if (braceCount <= 0 && current.trim()) {
        chunks.push(makeChunk(projectId, filePath, current, startLine, i + 1, chunkType, symbolName));
        current = '';
        inBlock = false;
        braceCount = 0;
      }
    } else if (/^(export\s+)?(const\s+\w+\s*=|let\s+\w+\s*=)/.test(trimmed)) {
      chunks.push(makeChunk(projectId, filePath, line, i + 1, i + 1, 'config', (trimmed.match(/(?:const|let)\s+(\w+)/) || [])[1] || ''));
    }
  }

  flushImports(lines.length + 1);

  if (current && inBlock) {
    chunks.push(makeChunk(projectId, filePath, current, startLine, lines.length, chunkType, symbolName));
  }

  return chunks.length > 0 ? chunks : [makeChunk(projectId, filePath, content, 1, lines.length, 'function', '')];
}

// --- Python chunking by indentation ---

function chunkPython(projectId: string, filePath: string, content: string): Chunk[] {
  const lines = content.split('\n');
  const chunks: Chunk[] = [];
  let current = '';
  let startLine = 1;
  let symbolName = '';
  let chunkType: ChunkType = 'function';
  let indentLevel = 0;
  let inBlock = false;
  let importBuffer = '';
  let importStartLine = 0;
  let inPyImportBlock = false;

  function flushImports(beforeLine: number) {
    if (importBuffer.trim()) {
      chunks.push(makeChunk(projectId, filePath, importBuffer, importStartLine, beforeLine - 1, 'config', 'imports'));
      importBuffer = '';
    }
    inPyImportBlock = false;
  }

  for (let i = 0; i < lines.length; i++) {
    const line = lines[i];
    const trimmed = line.trim();
    const indent = line.length - line.trimStart().length;

    // Multiline Python import continuation: from pkg import (\n  a,\n  b\n)
    if (inPyImportBlock) {
      importBuffer += line + '\n';
      if (trimmed.includes(')') || (!trimmed.endsWith('\\') && !trimmed.endsWith(','))) {
        inPyImportBlock = false;
      }
      continue;
    }

    if (!inBlock && isPyImportStart(trimmed)) {
      if (!importBuffer) importStartLine = i + 1;
      importBuffer += line + '\n';
      // Check for multiline: line has ( but no ) or ends with backslash
      if ((trimmed.includes('(') && !trimmed.includes(')')) || trimmed.endsWith('\\')) {
        inPyImportBlock = true;
      }
      continue;
    }

    if (importBuffer && !isPyImportStart(trimmed)) {
      flushImports(i + 1);
    }

    if (trimmed.startsWith('def ') || trimmed.startsWith('class ')) {
      if (current && inBlock) {
        chunks.push(makeChunk(projectId, filePath, current, startLine, i, chunkType, symbolName));
      }
      inBlock = true;
      chunkType = trimmed.startsWith('class ') ? 'class' : 'function';
      symbolName = (trimmed.match(/(?:def|class)\s+(\w+)/) || [])[1] || '';
      startLine = i + 1;
      current = line + '\n';
      indentLevel = indent;
    } else if (inBlock) {
      if (trimmed && indent <= indentLevel && !trimmed.startsWith('#')) {
        chunks.push(makeChunk(projectId, filePath, current, startLine, i, chunkType, symbolName));
        current = '';
        inBlock = false;
        i--; continue;
      }
      current += line + '\n';
    }
  }

  flushImports(lines.length + 1);

  if (current && inBlock) {
    chunks.push(makeChunk(projectId, filePath, current, startLine, lines.length, chunkType, symbolName));
  }

  return chunks.length > 0 ? chunks : [makeChunk(projectId, filePath, content, 1, lines.length, 'function', '')];
}

// --- Generic: 50-line windows ---

function chunkGeneric(projectId: string, filePath: string, content: string): Chunk[] {
  const lines = content.split('\n');
  const chunks: Chunk[] = [];
  const size = 50;
  for (let i = 0; i < lines.length; i += size) {
    const slice = lines.slice(i, i + size).join('\n');
    chunks.push(makeChunk(projectId, filePath, slice, i + 1, Math.min(i + size, lines.length), 'function', `chunk_${Math.floor(i / size)}`));
  }
  return chunks;
}

// --- Markdown: split by headers ---

function chunkMarkdown(projectId: string, filePath: string, content: string): Chunk[] {
  const lines = content.split('\n');
  const chunks: Chunk[] = [];
  let current = '';
  let startLine = 1;
  let symbolName = '';

  for (let i = 0; i < lines.length; i++) {
    if (lines[i].startsWith('#')) {
      if (current.trim()) chunks.push(makeChunk(projectId, filePath, current, startLine, i, 'documentation', symbolName));
      symbolName = lines[i].replace(/^#+\s*/, '');
      startLine = i + 1;
      current = lines[i] + '\n';
    } else {
      current += lines[i] + '\n';
    }
  }

  if (current.trim()) chunks.push(makeChunk(projectId, filePath, current, startLine, lines.length, 'documentation', symbolName));
  return chunks.length > 0 ? chunks : [makeChunk(projectId, filePath, content, 1, lines.length, 'documentation', '')];
}

// --- Main entry ---

/** Async chunk entry: tries AST chunking first, falls back to regex */
export async function chunkFileAST(projectId: string, filePath: string, content: string): Promise<Chunk[]> {
  const e = ext(filePath);

  // Try AST chunking for supported languages
  try {
    const { astChunkFile } = await import('./ast-chunker.js');
    const astChunks = await astChunkFile(projectId, filePath, content, e);
    if (astChunks && astChunks.length > 0) {
      return postProcess(astChunks);
    }
  } catch {
    // AST not available or failed — fall through to regex
  }

  // Regex fallback
  const chunks = chunkFileRegex(projectId, filePath, content);
  // Tag regex chunks
  for (const c of chunks) {
    c.chunk_source = 'regex';
  }
  return chunks;
}

/** Synchronous regex-only chunking (original behavior) */
export function chunkFile(projectId: string, filePath: string, content: string): Chunk[] {
  return chunkFileRegex(projectId, filePath, content);
}

function chunkFileRegex(projectId: string, filePath: string, content: string): Chunk[] {
  const e = ext(filePath);
  const lang = LANG_MAP[e];

  let chunks: Chunk[];
  if (lang === 'typescript' || lang === 'javascript') chunks = chunkJSTS(projectId, filePath, content);
  else if (lang === 'python') chunks = chunkPython(projectId, filePath, content);
  else if (lang === 'markdown') chunks = chunkMarkdown(projectId, filePath, content);
  else if (CODE_EXTENSIONS.has(e)) chunks = chunkGeneric(projectId, filePath, content);
  else {
    const paragraphs = content.split('\n\n').filter(p => p.trim());
    let lineNum = 1;
    chunks = paragraphs.map((p, i) => {
      const endLine = lineNum + p.split('\n').length - 1;
      const chunk = makeChunk(projectId, filePath, p, lineNum, endLine, 'documentation', `section_${i}`);
      lineNum = endLine + 2;
      return chunk;
    });
  }

  return postProcess(chunks);
}

function isFunctionDecl(line: string): boolean {
  // Exclude control-flow keywords that look like function calls
  if (/^(if|else|for|while|do|switch|catch)\s*[\s(]/.test(line.trim())) return false;
  return /^(export\s+)?(async\s+)?function\s+\w+/.test(line) ||
    /^(export\s+)?const\s+\w+\s*=\s*(?:\([^)]*\)\s*)?=>/.test(line) ||
    /^\w+\s*\([^)]*\)\s*\{/.test(line);
}

function isClassDecl(line: string): boolean {
  return /^(export\s+)?class\s+\w+/.test(line);
}

function extractFuncName(line: string): string {
  const m = line.match(/(?:function\s+(\w+)|const\s+(\w+)|(\w+)\s*\()/);
  return m ? (m[1] || m[2] || m[3]) : '';
}
