/**
 * Cross-File Symbol Resolver
 * Resolves symbol definitions and usages across the entire dependency graph
 */

import type {
  DependencyGraph,
  SymbolDefinition,
  SymbolUsage,
  ImportDeclaration,
  ExportDeclaration
} from '../types/cross-file-analysis.js';

export interface ResolvedSymbol {
  /** Symbol name */
  name: string;
  /** Definition location */
  definition: SymbolDefinition;
  /** All usage locations */
  usages: SymbolUsage[];
  /** Import chain (how symbol flows between files) */
  importChain: ImportChain[];
  /** Export locations */
  exports: ExportLocation[];
  /** Resolution confidence */
  confidence: number;
}

export interface ImportChain {
  /** File doing the importing */
  importingFile: string;
  /** File being imported from */
  importedFrom: string;
  /** Import declaration details */
  importDeclaration: ImportDeclaration;
  /** How the symbol is renamed/aliased */
  alias?: string;
}

export interface ExportLocation {
  /** File exporting the symbol */
  file: string;
  /** Export declaration details */
  exportDeclaration: ExportDeclaration;
  /** Whether it's re-exported from another file */
  reExportFrom?: string;
}

export interface SymbolResolutionResult {
  /** All resolved symbols */
  resolvedSymbols: Map<string, ResolvedSymbol>;
  /** Unresolved symbol usages */
  unresolvedUsages: SymbolUsage[];
  /** Symbol conflicts (multiple definitions) */
  conflicts: SymbolConflict[];
  /** Resolution statistics */
  stats: ResolutionStats;
}

export interface SymbolConflict {
  /** Symbol name */
  name: string;
  /** Conflicting definitions */
  definitions: SymbolDefinition[];
  /** Files where conflict occurs */
  conflictFiles: string[];
}

export interface ResolutionStats {
  /** Total symbols attempted to resolve */
  totalSymbols: number;
  /** Successfully resolved symbols */
  resolvedSymbols: number;
  /** Unresolved symbols */
  unresolvedSymbols: number;
  /** Symbols with conflicts */
  conflictedSymbols: number;
  /** Resolution accuracy percentage */
  resolutionAccuracy: number;
}

export class SymbolResolver {
  private graph: DependencyGraph;
  private resolvedSymbols: Map<string, ResolvedSymbol> = new Map();
  private symbolScopes: Map<string, Map<string, SymbolDefinition>> = new Map();

  constructor(graph: DependencyGraph) {
    this.graph = graph;
  }

  /**
   * Resolve all symbols in the dependency graph
   */
  resolveAllSymbols(): SymbolResolutionResult {
    console.log('🔍 Resolving cross-file symbols...');

    // Step 1: Build symbol scopes for each file
    this.buildSymbolScopes();
    console.log('📋 Built symbol scopes');

    // Step 2: Resolve imported symbols
    this.resolveImportedSymbols();
    console.log('📥 Resolved imported symbols');

    // Step 3: Trace symbol usage chains
    this.traceSymbolUsageChains();
    console.log('🔗 Traced symbol usage chains');

    // Step 4: Detect conflicts and generate results
    const result = this.generateResolutionResult();
    console.log(`✅ Resolution complete: ${result.stats.resolutionAccuracy}% accuracy`);

    return result;
  }

  /**
   * Resolve a specific symbol across the codebase
   */
  resolveSymbol(symbolName: string, contextFile?: string): ResolvedSymbol | null {
    // If already resolved, return cached result
    if (this.resolvedSymbols.has(symbolName)) {
      return this.resolvedSymbols.get(symbolName)!;
    }

    // Find all definitions of this symbol
    const definitions = this.graph.symbols.get(symbolName) || [];
    if (definitions.length === 0) {
      return null;
    }

    // Find all usages of this symbol
    const usages = this.graph.usages.get(symbolName) || [];

    // Build import chains for this symbol
    const importChains = this.buildImportChains(symbolName, contextFile);

    // Find export locations
    const exports = this.findExportLocations(symbolName);

    // Calculate resolution confidence
    const confidence = this.calculateResolutionConfidence(symbolName, definitions, usages, importChains);

    // Choose the most likely definition (if multiple)
    const primaryDefinition = this.selectPrimaryDefinition(definitions, contextFile);

    const resolvedSymbol: ResolvedSymbol = {
      name: symbolName,
      definition: primaryDefinition,
      usages,
      importChain: importChains,
      exports,
      confidence
    };

    this.resolvedSymbols.set(symbolName, resolvedSymbol);
    return resolvedSymbol;
  }

  /**
   * Find where a symbol is defined
   */
  findSymbolDefinition(symbolName: string, contextFile: string): SymbolDefinition | null {
    // First, check local scope
    const fileScope = this.symbolScopes.get(contextFile);
    if (fileScope?.has(symbolName)) {
      return fileScope.get(symbolName)!;
    }

    // Then check imported symbols
    const imports = this.graph.imports.get(contextFile) || [];
    for (const importDecl of imports) {
      if (importDecl.imported.includes(symbolName) && importDecl.resolvedPath) {
        const importedFileScope = this.symbolScopes.get(importDecl.resolvedPath);
        if (importedFileScope?.has(symbolName)) {
          return importedFileScope.get(symbolName)!;
        }
      }
    }

    // Finally, check global symbols
    const globalDefinitions = this.graph.symbols.get(symbolName) || [];
    return globalDefinitions.length > 0 ? globalDefinitions[0] : null;
  }

  /**
   * Find all files that use a specific symbol
   */
  findSymbolUsages(symbolName: string): SymbolUsage[] {
    return this.graph.usages.get(symbolName) || [];
  }

  /**
   * Trace how a symbol flows through imports/exports
   */
  traceSymbolFlow(symbolName: string, startFile: string): string[] {
    const flow: string[] = [startFile];
    const visited = new Set<string>([startFile]);

    const trace = (currentFile: string) => {
      const dependencies = this.graph.dependencies.get(currentFile) || [];
      
      for (const depFile of dependencies) {
        if (visited.has(depFile)) continue;

        const imports = this.graph.imports.get(currentFile) || [];
        const relevantImport = imports.find(imp => 
          imp.resolvedPath === depFile && imp.imported.includes(symbolName)
        );

        if (relevantImport) {
          visited.add(depFile);
          flow.push(depFile);
          trace(depFile); // Recursively trace further
        }
      }
    };

    trace(startFile);
    return flow;
  }

  private buildSymbolScopes(): void {
    for (const [filePath] of this.graph.files) {
      const fileSymbols = new Map<string, SymbolDefinition>();
      
      // Get all symbols defined in this file
      for (const [symbolName, definitions] of this.graph.symbols) {
        const fileDefinitions = definitions.filter(def => def.definedIn === filePath);
        if (fileDefinitions.length > 0) {
          // Use the first definition if multiple in same file
          fileSymbols.set(symbolName, fileDefinitions[0]);
        }
      }

      this.symbolScopes.set(filePath, fileSymbols);
    }
  }

  private resolveImportedSymbols(): void {
    for (const [filePath, imports] of this.graph.imports) {
      for (const importDecl of imports) {
        if (!importDecl.resolvedPath) continue;

        const exportedSymbols = this.graph.exports.get(importDecl.resolvedPath) || [];
        const fileScope = this.symbolScopes.get(filePath)!;

        // Map imported symbols to their definitions
        for (const importedSymbol of importDecl.imported) {
          // Check if the symbol is exported from the imported file
          const isExported = exportedSymbols.some(exp => 
            exp.exported.includes(importedSymbol) || 
            exp.exported.includes('default') && importedSymbol === 'default'
          );

          if (isExported) {
            const definition = this.findSymbolDefinition(importedSymbol, importDecl.resolvedPath);
            if (definition) {
              fileScope.set(importedSymbol, definition);
            }
          }
        }
      }
    }
  }

  private traceSymbolUsageChains(): void {
    for (const [symbolName, usages] of this.graph.usages) {
      for (const usage of usages) {
        // Find where this symbol comes from
        const definition = this.findSymbolDefinition(symbolName, usage.usedIn);
        if (definition) {
          // Update usage with import information
          const imports = this.graph.imports.get(usage.usedIn) || [];
          const relevantImport = imports.find(imp => imp.imported.includes(symbolName));
          if (relevantImport) {
            usage.importedFrom = relevantImport.source;
          }
        }
      }
    }
  }

  private buildImportChains(symbolName: string, contextFile?: string): ImportChain[] {
    const chains: ImportChain[] = [];
    const visited = new Set<string>();

    const buildChain = (currentFile: string) => {
      if (visited.has(currentFile)) return;
      visited.add(currentFile);

      const imports = this.graph.imports.get(currentFile) || [];
      for (const importDecl of imports) {
        if (importDecl.imported.includes(symbolName) && importDecl.resolvedPath) {
          chains.push({
            importingFile: currentFile,
            importedFrom: importDecl.resolvedPath,
            importDeclaration: importDecl
          });

          // Recursively build chain
          buildChain(importDecl.resolvedPath);
        }
      }
    };

    if (contextFile) {
      buildChain(contextFile);
    } else {
      // Build chains from all files that import this symbol
      for (const [filePath, imports] of this.graph.imports) {
        if (imports.some(imp => imp.imported.includes(symbolName))) {
          buildChain(filePath);
        }
      }
    }

    return chains;
  }

  private findExportLocations(symbolName: string): ExportLocation[] {
    const locations: ExportLocation[] = [];

    for (const [filePath, exports] of this.graph.exports) {
      for (const exportDecl of exports) {
        if (exportDecl.exported.includes(symbolName)) {
          locations.push({
            file: filePath,
            exportDeclaration: exportDecl,
            reExportFrom: exportDecl.source
          });
        }
      }
    }

    return locations;
  }

  private calculateResolutionConfidence(
    symbolName: string,
    definitions: SymbolDefinition[],
    usages: SymbolUsage[],
    importChains: ImportChain[]
  ): number {
    let confidence = 0;

    // Base confidence from having a definition
    if (definitions.length === 1) {
      confidence += 40; // Single definition is good
    } else if (definitions.length > 1) {
      confidence += 20; // Multiple definitions reduce confidence
    }

    // Confidence from usage patterns
    if (usages.length > 0) {
      confidence += Math.min(30, usages.length * 5); // More usage = more confidence
    }

    // Confidence from clear import chains
    if (importChains.length > 0) {
      confidence += Math.min(20, importChains.length * 10);
    }

    // Confidence from symbol naming patterns
    if (this.hasConsistentNaming(symbolName, definitions)) {
      confidence += 10;
    }

    return Math.min(100, confidence);
  }

  private selectPrimaryDefinition(definitions: SymbolDefinition[], contextFile?: string): SymbolDefinition {
    if (definitions.length === 1) {
      return definitions[0];
    }

    // Prefer definition in context file
    if (contextFile) {
      const contextDefinition = definitions.find(def => def.definedIn === contextFile);
      if (contextDefinition) {
        return contextDefinition;
      }
    }

    // Prefer exported definitions
    const exportedDefinitions = definitions.filter(def => {
      const exports = this.graph.exports.get(def.definedIn) || [];
      return exports.some(exp => exp.exported.includes(def.name));
    });

    if (exportedDefinitions.length > 0) {
      return exportedDefinitions[0];
    }

    // Default to first definition
    return definitions[0];
  }

  private hasConsistentNaming(symbolName: string, definitions: SymbolDefinition[]): boolean {
    // Check if all definitions use consistent naming patterns
    const names = definitions.map(def => def.name);
    return new Set(names).size === 1; // All definitions have same name
  }

  private generateResolutionResult(): SymbolResolutionResult {
    const unresolvedUsages: SymbolUsage[] = [];
    const conflicts: SymbolConflict[] = [];

    // Find unresolved usages
    for (const [symbolName, usages] of this.graph.usages) {
      const definitions = this.graph.symbols.get(symbolName) || [];
      if (definitions.length === 0) {
        unresolvedUsages.push(...usages);
      }
    }

    // Find conflicts
    for (const [symbolName, definitions] of this.graph.symbols) {
      if (definitions.length > 1) {
        const uniqueFiles = [...new Set(definitions.map(def => def.definedIn))];
        if (uniqueFiles.length > 1) {
          conflicts.push({
            name: symbolName,
            definitions,
            conflictFiles: uniqueFiles
          });
        }
      }
    }

    // Calculate statistics
    const totalSymbols = this.graph.symbols.size;
    const resolvedSymbols = this.resolvedSymbols.size;
    const unresolvedSymbols = totalSymbols - resolvedSymbols;
    const conflictedSymbols = conflicts.length;
    const resolutionAccuracy = totalSymbols > 0 ? Math.round((resolvedSymbols / totalSymbols) * 100) : 0;

    const stats: ResolutionStats = {
      totalSymbols,
      resolvedSymbols,
      unresolvedSymbols,
      conflictedSymbols,
      resolutionAccuracy
    };

    return {
      resolvedSymbols: this.resolvedSymbols,
      unresolvedUsages,
      conflicts,
      stats
    };
  }
}