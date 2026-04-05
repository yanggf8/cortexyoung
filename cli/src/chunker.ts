import { createHash } from 'crypto';

export type ChunkType = 'function' | 'class' | 'method' | 'documentation' | 'config';

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
}

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

function extractCalls(content: string): string[] {
  const matches = content.match(/(\w+)\s*\(/g);
  return matches ? [...new Set(matches.map(m => m.replace(/\s*\(/, '')))] : [];
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

// JS/TS chunking by brace counting
function chunkJSTS(projectId: string, filePath: string, content: string): Chunk[] {
  const lines = content.split('\n');
  const chunks: Chunk[] = [];
  let current = '';
  let startLine = 1;
  let symbolName = '';
  let chunkType: ChunkType = 'function';
  let braceCount = 0;
  let inBlock = false;

  for (let i = 0; i < lines.length; i++) {
    const line = lines[i];
    const trimmed = line.trim();

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
    } else if (/^(import\s+|export\s+|const\s+\w+\s*=|let\s+\w+\s*=)/.test(trimmed)) {
      chunks.push(makeChunk(projectId, filePath, line, i + 1, i + 1, 'config', (trimmed.match(/(?:import|export).*?(\w+)/) || [])[1] || ''));
    }
  }

  if (current && inBlock) {
    chunks.push(makeChunk(projectId, filePath, current, startLine, lines.length, chunkType, symbolName));
  }

  return chunks.length > 0 ? chunks : [makeChunk(projectId, filePath, content, 1, lines.length, 'function', '')];
}

// Python chunking by indentation
function chunkPython(projectId: string, filePath: string, content: string): Chunk[] {
  const lines = content.split('\n');
  const chunks: Chunk[] = [];
  let current = '';
  let startLine = 1;
  let symbolName = '';
  let chunkType: ChunkType = 'function';
  let indentLevel = 0;
  let inBlock = false;

  for (let i = 0; i < lines.length; i++) {
    const line = lines[i];
    const trimmed = line.trim();
    const indent = line.length - line.trimStart().length;

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

  if (current && inBlock) {
    chunks.push(makeChunk(projectId, filePath, current, startLine, lines.length, chunkType, symbolName));
  }

  return chunks.length > 0 ? chunks : [makeChunk(projectId, filePath, content, 1, lines.length, 'function', '')];
}

// Generic: 50-line windows
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

// Markdown: split by headers
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

export function chunkFile(projectId: string, filePath: string, content: string): Chunk[] {
  const e = ext(filePath);
  const lang = LANG_MAP[e];

  if (lang === 'typescript' || lang === 'javascript') return chunkJSTS(projectId, filePath, content);
  if (lang === 'python') return chunkPython(projectId, filePath, content);
  if (lang === 'markdown') return chunkMarkdown(projectId, filePath, content);
  if (CODE_EXTENSIONS.has(e)) return chunkGeneric(projectId, filePath, content);

  // Document files: paragraph chunking
  const paragraphs = content.split('\n\n').filter(p => p.trim());
  let lineNum = 1;
  return paragraphs.map((p, i) => {
    const endLine = lineNum + p.split('\n').length - 1;
    const chunk = makeChunk(projectId, filePath, p, lineNum, endLine, 'documentation', `section_${i}`);
    lineNum = endLine + 2;
    return chunk;
  });
}

function isFunctionDecl(line: string): boolean {
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
