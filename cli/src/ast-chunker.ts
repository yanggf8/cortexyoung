/**
 * Tree-sitter AST-based chunker.
 * Extracts function/class/method/interface/import/export nodes with
 * calls/imports/exports metadata from the AST.
 */

import { Parser, Language, Node } from 'web-tree-sitter';
import type { Chunk, ChunkType } from './chunker.js';
import { grammarForExtension, grammarPath, isGrammarAvailable } from './grammars.js';
import { createHash } from 'crypto';

let parserReady = false;

/** Must be called once before any AST chunking */
export async function initParser(): Promise<void> {
  if (parserReady) return;
  await Parser.init();
  parserReady = true;
}

// Cache loaded languages to avoid re-reading WASM files
const languageCache = new Map<string, Language>();

async function loadLanguage(ext: string): Promise<Language | null> {
  const grammar = grammarForExtension(ext);
  if (!grammar) return null;

  const cached = languageCache.get(grammar.name);
  if (cached) return cached;

  const wasmPath = await grammarPath(grammar);
  if (!wasmPath) return null;

  const lang = await Language.load(wasmPath);
  languageCache.set(grammar.name, lang);
  return lang;
}

/** Check if AST chunking is available for a given file extension */
export async function canASTChunk(ext: string): Promise<boolean> {
  const grammar = grammarForExtension(ext);
  if (!grammar) return false;
  return isGrammarAvailable(grammar);
}

function hashContent(content: string): string {
  return createHash('sha256').update(content).digest('hex');
}

const LANG_MAP: Record<string, string> = {
  '.ts': 'typescript', '.tsx': 'typescript',
  '.js': 'javascript', '.jsx': 'javascript',
  '.py': 'python',
};

// --- Node type classification ---

const JS_CHUNK_TYPES = new Set([
  'function_declaration',
  'generator_function_declaration',
  'class_declaration',
  'abstract_class_declaration',
  'interface_declaration',
  'type_alias_declaration',
  'enum_declaration',
]);

const JS_EXPORT_TYPES = new Set([
  'export_statement',
]);

const JS_IMPORT_TYPES = new Set([
  'import_statement',
]);

const PY_CHUNK_TYPES = new Set([
  'function_definition',
  'class_definition',
  'decorated_definition',
]);

const PY_IMPORT_TYPES = new Set([
  'import_statement',
  'import_from_statement',
]);

// --- AST extraction ---

interface ExtractedNode {
  type: ChunkType;
  symbolName: string;
  content: string;
  startLine: number; // 1-based
  endLine: number;   // 1-based
  calls: string[];
  imports: string[];
  exports: string[];
}

function extractCallsFromAST(node: Node): string[] {
  const calls = new Set<string>();
  const CONTROL_FLOW = new Set(['if', 'else', 'for', 'while', 'do', 'switch', 'catch', 'return', 'throw', 'new', 'typeof', 'instanceof', 'delete', 'void']);

  function walk(n: Node) {
    if (n.type === 'call_expression' || n.type === 'call') {
      const fn = n.childForFieldName('function') ?? n.firstNamedChild;
      if (fn) {
        const name = extractCallName(fn);
        if (name && !CONTROL_FLOW.has(name)) calls.add(name);
      }
    }
    for (const child of n.namedChildren) {
      walk(child);
    }
  }
  walk(node);
  return [...calls];
}

function extractCallName(node: Node): string {
  if (node.type === 'identifier' || node.type === 'property_identifier') {
    return node.text;
  }
  if (node.type === 'member_expression' || node.type === 'attribute') {
    const prop = node.childForFieldName('property') ?? node.lastNamedChild;
    return prop ? prop.text : '';
  }
  return '';
}

function extractImportsFromAST(node: Node, lang: string): string[] {
  const imports: string[] = [];

  function walk(n: Node) {
    if (lang === 'python') {
      if (n.type === 'import_statement') {
        for (const child of n.namedChildren) {
          if (child.type === 'dotted_name') imports.push(child.text);
          if (child.type === 'aliased_import') {
            const name = child.childForFieldName('name');
            if (name) imports.push(name.text);
          }
        }
      } else if (n.type === 'import_from_statement') {
        const module = n.childForFieldName('module_name');
        if (module) imports.push(module.text);
      }
    } else {
      if (n.type === 'import_statement') {
        const source = n.childForFieldName('source');
        if (source) {
          imports.push(source.text.replace(/['"]/g, ''));
        }
      }
    }

    for (const child of n.namedChildren) {
      walk(child);
    }
  }
  walk(node);
  return imports;
}

function extractExportsFromAST(node: Node): string[] {
  const exports: string[] = [];

  function walk(n: Node) {
    if (n.type === 'export_statement') {
      const decl = n.childForFieldName('declaration');
      if (decl) {
        const name = extractDeclName(decl);
        if (name) exports.push(name);
      }
      for (const child of n.namedChildren) {
        if (child.type === 'export_clause') {
          for (const spec of child.namedChildren) {
            if (spec.type === 'export_specifier') {
              const name = spec.childForFieldName('name') ?? spec.firstNamedChild;
              if (name) exports.push(name.text);
            }
          }
        }
      }
    }
    for (const child of n.namedChildren) {
      walk(child);
    }
  }
  walk(node);
  return exports;
}

function extractDeclName(node: Node): string {
  switch (node.type) {
    case 'function_declaration':
    case 'generator_function_declaration':
    case 'class_declaration':
    case 'abstract_class_declaration':
    case 'interface_declaration':
    case 'type_alias_declaration':
    case 'enum_declaration': {
      const name = node.childForFieldName('name');
      return name ? name.text : '';
    }
    case 'lexical_declaration':
    case 'variable_declaration': {
      const declarator = node.namedChildren.find((c: Node) => c.type === 'variable_declarator');
      if (declarator) {
        const name = declarator.childForFieldName('name');
        return name ? name.text : '';
      }
      return '';
    }
    case 'function_definition':
    case 'class_definition': {
      const name = node.childForFieldName('name');
      return name ? name.text : '';
    }
    case 'decorated_definition': {
      const definition = node.childForFieldName('definition');
      return definition ? extractDeclName(definition) : '';
    }
    default:
      return '';
  }
}

function classifyChunkType(nodeType: string): ChunkType {
  if (nodeType.includes('class') || nodeType === 'abstract_class_declaration') return 'class';
  if (nodeType.includes('function') || nodeType === 'generator_function_declaration') return 'function';
  if (nodeType === 'method_definition' || nodeType === 'function_definition') return 'function';
  if (nodeType === 'interface_declaration' || nodeType === 'type_alias_declaration' || nodeType === 'enum_declaration') return 'config';
  if (nodeType === 'decorated_definition') return 'function';
  return 'function';
}

/**
 * Detect if a lexical_declaration/variable_declaration initializes to a function value.
 * e.g., `const foo = () => {}` or `const bar = function() {}`.
 */
function isFunctionValuedDeclaration(decl: Node): boolean {
  if (decl.type !== 'lexical_declaration' && decl.type !== 'variable_declaration') return false;
  const declarator = decl.namedChildren.find((c: Node) => c.type === 'variable_declarator');
  if (!declarator) return false;
  const value = declarator.childForFieldName('value');
  if (!value) return false;
  return (
    value.type === 'arrow_function' ||
    value.type === 'function_expression' ||
    value.type === 'function' ||
    value.type === 'generator_function'
  );
}

function classifyPyChunkType(node: Node): ChunkType {
  if (node.type === 'class_definition') return 'class';
  if (node.type === 'function_definition') return 'function';
  if (node.type === 'decorated_definition') {
    const inner = node.childForFieldName('definition');
    if (inner) return classifyPyChunkType(inner);
    return 'function';
  }
  return 'function';
}

// --- JS/TS extraction ---

function extractJSTS(root: Node): ExtractedNode[] {
  const nodes: ExtractedNode[] = [];
  const importNodes: Node[] = [];

  for (const child of root.namedChildren) {
    if (JS_IMPORT_TYPES.has(child.type)) {
      importNodes.push(child);
      continue;
    }

    if (JS_EXPORT_TYPES.has(child.type)) {
      const decl = child.childForFieldName('declaration');
      if (decl && (JS_CHUNK_TYPES.has(decl.type) || decl.type === 'lexical_declaration' || decl.type === 'variable_declaration')) {
        const name = extractDeclName(decl);
        const chunkType: ChunkType = JS_CHUNK_TYPES.has(decl.type)
          ? classifyChunkType(decl.type)
          : isFunctionValuedDeclaration(decl)
            ? 'function'
            : 'config';
        nodes.push({
          type: chunkType,
          symbolName: name,
          content: child.text,
          startLine: child.startPosition.row + 1,
          endLine: child.endPosition.row + 1,
          calls: extractCallsFromAST(child),
          imports: [],
          exports: name ? [name] : [],
        });
        continue;
      }
      nodes.push({
        type: 'config',
        symbolName: 'exports',
        content: child.text,
        startLine: child.startPosition.row + 1,
        endLine: child.endPosition.row + 1,
        calls: extractCallsFromAST(child),
        imports: [],
        exports: extractExportsFromAST(child),
      });
      continue;
    }

    if (JS_CHUNK_TYPES.has(child.type)) {
      const name = extractDeclName(child);
      nodes.push({
        type: classifyChunkType(child.type),
        symbolName: name,
        content: child.text,
        startLine: child.startPosition.row + 1,
        endLine: child.endPosition.row + 1,
        calls: extractCallsFromAST(child),
        imports: [],
        exports: [],
      });
      continue;
    }

    if (child.type === 'lexical_declaration' || child.type === 'variable_declaration') {
      const name = extractDeclName(child);
      nodes.push({
        type: isFunctionValuedDeclaration(child) ? 'function' : 'config',
        symbolName: name,
        content: child.text,
        startLine: child.startPosition.row + 1,
        endLine: child.endPosition.row + 1,
        calls: extractCallsFromAST(child),
        imports: [],
        exports: [],
      });
      continue;
    }

    if (child.type === 'expression_statement') {
      const expr = child.firstNamedChild;
      if (expr && (expr.type === 'call_expression' || expr.type === 'assignment_expression')) {
        nodes.push({
          type: 'config',
          symbolName: extractCallName(expr.childForFieldName('function') ?? expr.firstNamedChild ?? expr),
          content: child.text,
          startLine: child.startPosition.row + 1,
          endLine: child.endPosition.row + 1,
          calls: extractCallsFromAST(child),
          imports: [],
          exports: [],
        });
      }
    }
  }

  if (importNodes.length > 0) {
    const firstImport = importNodes[0];
    const lastImport = importNodes[importNodes.length - 1];
    const importContent = importNodes.map(n => n.text).join('\n');
    const allImports: string[] = [];
    for (const n of importNodes) {
      allImports.push(...extractImportsFromAST(n, 'javascript'));
    }
    nodes.unshift({
      type: 'config',
      symbolName: 'imports',
      content: importContent,
      startLine: firstImport.startPosition.row + 1,
      endLine: lastImport.endPosition.row + 1,
      imports: allImports,
      exports: [],
      calls: [],
    });
  }

  return nodes;
}

// --- Python extraction ---

function extractPython(root: Node): ExtractedNode[] {
  const nodes: ExtractedNode[] = [];
  const importNodes: Node[] = [];

  for (const child of root.namedChildren) {
    if (PY_IMPORT_TYPES.has(child.type)) {
      importNodes.push(child);
      continue;
    }

    if (PY_CHUNK_TYPES.has(child.type)) {
      const name = extractDeclName(child);
      nodes.push({
        type: classifyPyChunkType(child),
        symbolName: name,
        content: child.text,
        startLine: child.startPosition.row + 1,
        endLine: child.endPosition.row + 1,
        calls: extractCallsFromAST(child),
        imports: [],
        exports: [],
      });
      continue;
    }

    if (child.type === 'expression_statement') {
      const expr = child.firstNamedChild;
      if (expr && expr.type === 'assignment') {
        const left = expr.childForFieldName('left');
        nodes.push({
          type: 'config',
          symbolName: left ? left.text : '',
          content: child.text,
          startLine: child.startPosition.row + 1,
          endLine: child.endPosition.row + 1,
          calls: extractCallsFromAST(child),
          imports: [],
          exports: [],
        });
      }
    }
  }

  if (importNodes.length > 0) {
    const first = importNodes[0];
    const last = importNodes[importNodes.length - 1];
    const importContent = importNodes.map(n => n.text).join('\n');
    const allImports: string[] = [];
    for (const n of importNodes) {
      allImports.push(...extractImportsFromAST(n, 'python'));
    }
    nodes.unshift({
      type: 'config',
      symbolName: 'imports',
      content: importContent,
      startLine: first.startPosition.row + 1,
      endLine: last.endPosition.row + 1,
      imports: allImports,
      exports: [],
      calls: [],
    });
  }

  return nodes;
}

// --- Main entry ---

/**
 * Parse a file using tree-sitter and extract structured chunks.
 * Returns null if AST chunking is not available for this file type.
 */
export async function astChunkFile(
  projectId: string,
  filePath: string,
  content: string,
  ext: string
): Promise<Chunk[] | null> {
  if (!parserReady) return null;

  const language = await loadLanguage(ext);
  if (!language) return null;

  const lang = LANG_MAP[ext];
  if (!lang) return null;

  const parser = new Parser();
  parser.setLanguage(language);

  const tree = parser.parse(content);
  if (!tree) {
    parser.delete();
    return null;
  }

  const root = tree.rootNode;
  let extracted: ExtractedNode[];

  if (lang === 'python') {
    extracted = extractPython(root);
  } else {
    extracted = extractJSTS(root);
  }

  tree.delete();
  parser.delete();

  if (extracted.length === 0) return null;

  const chunks: Chunk[] = extracted.map(node => ({
    chunk_id: `${projectId}:${filePath}:${node.startLine}`,
    project_id: projectId,
    file_path: filePath,
    symbol_name: node.symbolName || undefined,
    chunk_type: node.type,
    start_line: node.startLine,
    end_line: node.endLine,
    content: node.content.trim(),
    content_hash: hashContent(node.content),
    language: lang,
    calls: node.calls,
    imports: node.imports,
    exports: node.exports,
    chunk_source: 'ast' as const,
  }));

  return chunks;
}
