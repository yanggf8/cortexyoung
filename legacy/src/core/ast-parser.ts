/**
 * AST-based Import/Export Parser
 * Parses TypeScript/JavaScript files to extract imports, exports, and symbol definitions
 */

import { readFileSync } from 'fs';
import { dirname, resolve, join } from 'path';
import { parse } from '@typescript-eslint/typescript-estree';
import type { TSESTree } from '@typescript-eslint/typescript-estree';
import type {
  FileNode,
  ImportDeclaration,
  ExportDeclaration,
  SymbolDefinition,
  SymbolUsage,
  FileType,
  SymbolType,
  SymbolContext,
  UsageContext
} from '../types/cross-file-analysis.js';

export class ASTParser {
  private projectRoot: string;
  private parsedFiles: Map<string, TSESTree.Program> = new Map();

  constructor(projectRoot: string) {
    this.projectRoot = projectRoot;
  }

  /**
   * Parse a file and extract all cross-file analysis information
   */
  async parseFile(filePath: string): Promise<FileNode> {
    const content = readFileSync(filePath, 'utf8');
    const relativePath = filePath.replace(this.projectRoot, '').replace(/^\//, '');
    
    let ast: TSESTree.Program | undefined;
    try {
      ast = parse(content, {
        loc: true,
        range: true,
        tokens: true,
        comment: true,
        jsx: filePath.endsWith('.jsx') || filePath.endsWith('.tsx'),
      });
      this.parsedFiles.set(filePath, ast);
    } catch (error) {
      console.warn(`Failed to parse ${filePath}:`, error);
    }

    return {
      path: filePath,
      relativePath,
      content,
      ast,
      lastModified: Date.now(),
      fileType: this.classifyFileType(filePath, content, ast)
    };
  }

  /**
   * Extract import declarations from AST
   */
  extractImports(filePath: string, ast: TSESTree.Program): ImportDeclaration[] {
    const imports: ImportDeclaration[] = [];

    for (const node of ast.body) {
      if (node.type === 'ImportDeclaration') {
        const importDecl = this.parseImportDeclaration(node, filePath);
        if (importDecl) {
          imports.push(importDecl);
        }
      }
    }

    return imports;
  }

  /**
   * Extract export declarations from AST
   */
  extractExports(ast: TSESTree.Program): ExportDeclaration[] {
    const exports: ExportDeclaration[] = [];

    for (const node of ast.body) {
      if (node.type === 'ExportDefaultDeclaration') {
        exports.push({
          exported: ['default'],
          exportType: 'default',
          lineNumber: node.loc?.start.line || 0
        });
      } else if (node.type === 'ExportNamedDeclaration') {
        const exportDecl = this.parseNamedExportDeclaration(node);
        if (exportDecl) {
          exports.push(exportDecl);
        }
      } else if (node.type === 'ExportAllDeclaration') {
        if (node.source) {
          exports.push({
            exported: ['*'],
            exportType: 'named',
            lineNumber: node.loc?.start.line || 0,
            source: node.source.value as string
          });
        }
      }
    }

    return exports;
  }

  /**
   * Extract symbol definitions from AST
   */
  extractSymbolDefinitions(filePath: string, ast: TSESTree.Program): SymbolDefinition[] {
    const symbols: SymbolDefinition[] = [];

    const extractFromNode = (node: TSESTree.Node, context: SymbolContext = 'definition') => {
      switch (node.type) {
        case 'FunctionDeclaration':
          if (node.id?.name) {
            symbols.push({
              name: node.id.name,
              type: 'function',
              definedIn: filePath,
              lineNumber: node.loc?.start.line || 0,
              astNode: node,
              context
            });
          }
          break;

        case 'ClassDeclaration':
          if (node.id?.name) {
            symbols.push({
              name: node.id.name,
              type: 'class',
              definedIn: filePath,
              lineNumber: node.loc?.start.line || 0,
              astNode: node,
              context
            });
          }
          break;

        case 'VariableDeclaration':
          for (const declarator of node.declarations) {
            if (declarator.id.type === 'Identifier') {
              const symbolType = node.kind === 'const' ? 'constant' : 'variable';
              symbols.push({
                name: declarator.id.name,
                type: symbolType,
                definedIn: filePath,
                lineNumber: declarator.loc?.start.line || 0,
                astNode: declarator,
                context
              });
            }
          }
          break;

        case 'TSInterfaceDeclaration':
          symbols.push({
            name: node.id.name,
            type: 'interface',
            definedIn: filePath,
            lineNumber: node.loc?.start.line || 0,
            astNode: node,
            context
          });
          break;

        case 'TSTypeAliasDeclaration':
          symbols.push({
            name: node.id.name,
            type: 'type',
            definedIn: filePath,
            lineNumber: node.loc?.start.line || 0,
            astNode: node,
            context
          });
          break;

        case 'TSEnumDeclaration':
          symbols.push({
            name: node.id.name,
            type: 'enum',
            definedIn: filePath,
            lineNumber: node.loc?.start.line || 0,
            astNode: node,
            context
          });
          break;
      }
    };

    // Walk the AST to find all symbol definitions
    this.walkAST(ast, extractFromNode);

    return symbols;
  }

  /**
   * Extract symbol usages from AST
   */
  extractSymbolUsages(filePath: string, ast: TSESTree.Program): SymbolUsage[] {
    const usages: SymbolUsage[] = [];

    const extractUsage = (node: TSESTree.Node) => {
      switch (node.type) {
        case 'CallExpression':
          if (node.callee.type === 'Identifier') {
            usages.push({
              name: node.callee.name,
              usedIn: filePath,
              lineNumber: node.loc?.start.line || 0,
              context: 'call'
            });
          } else if (node.callee.type === 'MemberExpression' && 
                     node.callee.property.type === 'Identifier') {
            usages.push({
              name: node.callee.property.name,
              usedIn: filePath,
              lineNumber: node.loc?.start.line || 0,
              context: 'method_call'
            });
          }
          break;

        case 'MemberExpression':
          if (node.property.type === 'Identifier') {
            usages.push({
              name: node.property.name,
              usedIn: filePath,
              lineNumber: node.loc?.start.line || 0,
              context: 'property_access'
            });
          }
          break;

        case 'Identifier':
          // Only count identifiers that are not part of declarations
          const parent = (node as any).parent;
          if (parent && !this.isDeclarationContext(parent, node)) {
            usages.push({
              name: node.name,
              usedIn: filePath,
              lineNumber: node.loc?.start.line || 0,
              context: 'assignment'
            });
          }
          break;
      }
    };

    this.walkAST(ast, extractUsage);

    return usages;
  }

  /**
   * Resolve import path to absolute path
   */
  resolveImportPath(importPath: string, currentFile: string): string | undefined {
    try {
      // Handle relative imports
      if (importPath.startsWith('.')) {
        const currentDir = dirname(currentFile);
        const resolved = resolve(currentDir, importPath);
        
        // Try different extensions
        const extensions = ['.ts', '.tsx', '.js', '.jsx', '.json'];
        for (const ext of extensions) {
          const withExt = resolved + ext;
          try {
            readFileSync(withExt);
            return withExt;
          } catch {}
        }
        
        // Try index files
        for (const ext of extensions) {
          const indexFile = join(resolved, 'index' + ext);
          try {
            readFileSync(indexFile);
            return indexFile;
          } catch {}
        }
      }
      
      // For now, skip node_modules resolution (could be added later)
      return undefined;
    } catch {
      return undefined;
    }
  }

  private parseImportDeclaration(node: TSESTree.ImportDeclaration, filePath: string): ImportDeclaration | null {
    if (!node.source?.value) return null;

    const imported: string[] = [];
    let importType: 'default' | 'named' | 'namespace' | 'mixed' = 'named';

    if (node.specifiers.length === 0) {
      // Side-effect import: import './module'
      importType = 'named';
    } else {
      let hasDefault = false;
      let hasNamed = false;
      let hasNamespace = false;

      for (const specifier of node.specifiers) {
        switch (specifier.type) {
          case 'ImportDefaultSpecifier':
            imported.push('default');
            hasDefault = true;
            break;
          case 'ImportSpecifier':
            imported.push(specifier.imported.type === 'Identifier' ? specifier.imported.name : specifier.imported.value);
            hasNamed = true;
            break;
          case 'ImportNamespaceSpecifier':
            imported.push('*');
            hasNamespace = true;
            break;
        }
      }

      if (hasNamespace) {
        importType = 'namespace';
      } else if (hasDefault && hasNamed) {
        importType = 'mixed';
      } else if (hasDefault) {
        importType = 'default';
      } else {
        importType = 'named';
      }
    }

    const source = node.source.value as string;
    const resolvedPath = this.resolveImportPath(source, filePath);

    return {
      imported,
      source,
      resolvedPath,
      importType,
      lineNumber: node.loc?.start.line || 0
    };
  }

  private parseNamedExportDeclaration(node: TSESTree.ExportNamedDeclaration): ExportDeclaration | null {
    const exported: string[] = [];

    if (node.declaration) {
      // export const foo = ..., export function bar() {}, etc.
      switch (node.declaration.type) {
        case 'VariableDeclaration':
          for (const declarator of node.declaration.declarations) {
            if (declarator.id.type === 'Identifier') {
              exported.push(declarator.id.name);
            }
          }
          break;
        case 'FunctionDeclaration':
        case 'ClassDeclaration':
        case 'TSInterfaceDeclaration':
        case 'TSTypeAliasDeclaration':
        case 'TSEnumDeclaration':
          if (node.declaration.id?.name) {
            exported.push(node.declaration.id.name);
          }
          break;
      }
    } else if (node.specifiers) {
      // export { foo, bar } from './module'
      for (const specifier of node.specifiers) {
        if (specifier.type === 'ExportSpecifier') {
          exported.push(specifier.exported.type === 'Identifier' ? specifier.exported.name : specifier.exported.value);
        }
      }
    }

    if (exported.length === 0) return null;

    return {
      exported,
      exportType: 'named',
      lineNumber: node.loc?.start.line || 0,
      source: node.source?.value as string
    };
  }

  private classifyFileType(filePath: string, content: string, ast?: TSESTree.Program): FileType {
    const fileName = filePath.toLowerCase();
    const baseName = fileName.split('/').pop() || '';

    // File name based classification
    if (baseName.includes('route') || baseName.includes('router')) return 'route';
    if (baseName.includes('controller')) return 'controller';
    if (baseName.includes('middleware')) return 'middleware';
    if (baseName.includes('service')) return 'service';
    if (baseName.includes('model')) return 'model';
    if (baseName.includes('util') || baseName.includes('helper')) return 'utility';
    if (baseName.includes('config')) return 'config';
    if (baseName.includes('test') || baseName.includes('spec')) return 'test';
    if (['index', 'main', 'app', 'server'].some(name => baseName.startsWith(name))) return 'main';

    // Content based classification
    if (content.includes('app.get') || content.includes('router.')) return 'route';
    if (content.includes('req, res') && content.includes('json')) return 'controller';
    if (content.includes('app.use') && content.includes('next')) return 'middleware';
    if (content.includes('class') && content.includes('Service')) return 'service';
    if (content.includes('Schema') || content.includes('model')) return 'model';

    return 'unknown';
  }

  private walkAST(node: TSESTree.Node | TSESTree.Program, callback: (node: TSESTree.Node) => void) {
    callback(node);

    // Walk child nodes
    for (const key in node) {
      if (key === 'parent' || key === 'range' || key === 'loc') continue;
      
      const child = (node as any)[key];
      if (Array.isArray(child)) {
        for (const item of child) {
          if (item && typeof item === 'object' && item.type) {
            this.walkAST(item, callback);
          }
        }
      } else if (child && typeof child === 'object' && child.type) {
        this.walkAST(child, callback);
      }
    }
  }

  private isDeclarationContext(parent: TSESTree.Node, identifier: TSESTree.Identifier): boolean {
    // Check if this identifier is being declared rather than used
    switch (parent.type) {
      case 'FunctionDeclaration':
      case 'ClassDeclaration':
        return parent.id === identifier;
      case 'VariableDeclarator':
        return parent.id === identifier;
      case 'Property':
        return parent.key === identifier && !parent.computed;
      case 'ImportDefaultSpecifier':
      case 'ImportSpecifier':
      case 'ImportNamespaceSpecifier':
        return true;
      default:
        return false;
    }
  }
}