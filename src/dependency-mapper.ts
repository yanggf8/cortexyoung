import * as path from 'path';
import * as fs from 'fs/promises';
import { log, warn } from './logging-utils';
import {
  CodeSymbol,
  CodeRelationship,
  RelationshipGraph,
  ImportDeclaration,
  ExportDeclaration,
  RelationshipType
} from './relationship-types';
import { CallGraphAnalyzer } from './call-graph-analyzer';

export interface ModuleInfo {
  filePath: string;
  exports: Map<string, ExportInfo>;
  imports: Map<string, ImportInfo>;
  symbols: Map<string, CodeSymbol>;
  isExternal: boolean;           // node_modules, CDN, etc.
  packageName?: string;          // For external modules
  version?: string;              // For external modules
}

export interface ExportInfo {
  name: string;
  type: 'default' | 'named' | 'namespace';
  symbol: CodeSymbol;
  isReExport: boolean;
  originalModule?: string;       // For re-exports
}

export interface ImportInfo {
  name: string;
  type: 'default' | 'named' | 'namespace' | 'dynamic';
  source: string;
  resolvedPath?: string;
  isTypeOnly: boolean;
  usageCount: number;            // How often this import is used
  usageLocations: number[];      // Line numbers where used
}

export interface DependencyChain {
  modules: string[];             // Ordered list of module paths
  relationships: string[];       // Ordered list of relationship IDs
  chainType: 'linear' | 'circular' | 'tree';
  depth: number;
  strength: number;              // Average relationship strength
  description: string;
}

export interface CircularDependency {
  modules: string[];
  entryPoint: string;
  severity: 'warning' | 'error';
  suggestedFix?: string;
}

export class DependencyMapper {
  private moduleMap: Map<string, ModuleInfo> = new Map();
  private dependencyGraph: Map<string, Set<string>> = new Map();
  private reverseDependencyGraph: Map<string, Set<string>> = new Map();
  private externalModules: Map<string, ModuleInfo> = new Map();
  private callGraphAnalyzer: CallGraphAnalyzer;
  private repositoryPath: string;

  constructor(repositoryPath: string) {
    this.repositoryPath = repositoryPath;
    this.callGraphAnalyzer = new CallGraphAnalyzer();
  }

  async buildDependencyMap(files: Map<string, string>): Promise<void> {
    log(`[DependencyMapper] Building dependency map files=${files.size}`);
    
    // Phase 1: Analyze all files and extract imports/exports
    for (const [filePath, content] of files) {
      await this.analyzeModuleDependencies(filePath, content);
    }

    // Phase 2: Resolve import paths to actual modules
    await this.resolveImportPaths();

    // Phase 3: Build dependency graph
    this.buildDependencyGraph();

    // Phase 4: Detect circular dependencies
    const circularDeps = this.detectCircularDependencies();
    if (circularDeps.length > 0) {
      warn(`[DependencyMapper] Found circular dependencies count=${circularDeps.length}`);
      circularDeps.forEach(dep => {
        warn(`[DependencyMapper] Circular dependency: ${dep.modules.join(' → ')}`);
      });
    }

    log(`[DependencyMapper] Dependency map built internal=${this.moduleMap.size} external=${this.externalModules.size}`);
  }

  private async analyzeModuleDependencies(filePath: string, content: string): Promise<void> {
    try {
      const analysis = await this.callGraphAnalyzer.analyzeFile(filePath, content);
      
      const moduleInfo: ModuleInfo = {
        filePath,
        exports: new Map(),
        imports: new Map(),
        symbols: new Map(),
        isExternal: false
      };

      // Process exports
      analysis.exports.forEach(exportDecl => {
        const symbol = analysis.symbols.find(s => s.name === exportDecl.exportedSymbol);
        if (symbol) {
          const exportInfo: ExportInfo = {
            name: exportDecl.exportedSymbol,
            type: exportDecl.exportType,
            symbol,
            isReExport: exportDecl.isReExport || false,
            originalModule: exportDecl.originalSource
          };
          
          moduleInfo.exports.set(exportDecl.exportedSymbol, exportInfo);
        }
      });

      // Process imports
      analysis.imports.forEach(importDecl => {
        const importInfo: ImportInfo = {
          name: importDecl.importedSymbols.join(', '),
          type: importDecl.importType,
          source: importDecl.source,
          isTypeOnly: importDecl.isTypeOnly || false,
          usageCount: 0,
          usageLocations: []
        };

        // Analyze usage of imported symbols
        this.analyzeImportUsage(content, importDecl, importInfo);
        
        moduleInfo.imports.set(importDecl.source, importInfo);
      });

      // Store symbols
      analysis.symbols.forEach(symbol => {
        moduleInfo.symbols.set(symbol.name, symbol);
      });

      this.moduleMap.set(filePath, moduleInfo);

    } catch (error) {
      warn(`[DependencyMapper] Failed to analyze dependencies for ${filePath} error=${error instanceof Error ? error.message : error}`);
    }
  }

  private analyzeImportUsage(content: string, importDecl: ImportDeclaration, importInfo: ImportInfo): void {
    const lines = content.split('\n');
    
    importDecl.importedSymbols.forEach(symbolName => {
      lines.forEach((line, index) => {
        // Skip the import line itself
        if (index + 1 === importDecl.line) return;
        
        // Simple usage detection (can be improved with AST analysis)
        const usageRegex = new RegExp(`\\b${symbolName}\\b`, 'g');
        const matches = line.match(usageRegex);
        
        if (matches) {
          importInfo.usageCount += matches.length;
          importInfo.usageLocations.push(index + 1);
        }
      });
    });
  }

  private async resolveImportPaths(): Promise<void> {
    for (const [filePath, moduleInfo] of this.moduleMap) {
      for (const [source, importInfo] of moduleInfo.imports) {
        const resolvedPath = await this.resolveModulePath(source, filePath);
        importInfo.resolvedPath = resolvedPath;

        // Handle external modules
        if (this.isExternalModule(source)) {
          await this.handleExternalModule(source, resolvedPath);
        }
      }
    }
  }

  private async resolveModulePath(source: string, fromFile: string): Promise<string | undefined> {
    // Handle relative imports
    if (source.startsWith('./') || source.startsWith('../')) {
      const fromDir = path.dirname(fromFile);
      const resolvedPath = path.resolve(fromDir, source);
      
      // Try different extensions
      const extensions = ['.js', '.ts', '.jsx', '.tsx', '.json'];
      for (const ext of extensions) {
        const fullPath = resolvedPath + ext;
        try {
          await fs.access(fullPath);
          return fullPath;
        } catch {
          // Try with index file
          const indexPath = path.join(resolvedPath, 'index' + ext);
          try {
            await fs.access(indexPath);
            return indexPath;
          } catch {
            continue;
          }
        }
      }
    }

    // Handle absolute imports (node_modules, etc.)
    if (!source.startsWith('.')) {
      // For external modules, return the package name
      const packageName = source.split('/')[0];
      return `node_modules/${packageName}`;
    }

    return undefined;
  }

  private isExternalModule(source: string): boolean {
    return !source.startsWith('./') && !source.startsWith('../') && !path.isAbsolute(source);
  }

  private async handleExternalModule(source: string, resolvedPath?: string): Promise<void> {
    const packageName = source.split('/')[0];
    
    if (!this.externalModules.has(packageName)) {
      const moduleInfo: ModuleInfo = {
        filePath: resolvedPath || `node_modules/${packageName}`,
        exports: new Map(),
        imports: new Map(),
        symbols: new Map(),
        isExternal: true,
        packageName
      };

      // Try to read package.json for version info
      try {
        const packageJsonPath = path.join(this.repositoryPath, 'node_modules', packageName, 'package.json');
        const packageJson = JSON.parse(await fs.readFile(packageJsonPath, 'utf-8'));
        moduleInfo.version = packageJson.version;
      } catch {
        // Package.json not found or readable
      }

      this.externalModules.set(packageName, moduleInfo);
    }
  }

  private buildDependencyGraph(): void {
    for (const [filePath, moduleInfo] of this.moduleMap) {
      if (!this.dependencyGraph.has(filePath)) {
        this.dependencyGraph.set(filePath, new Set());
      }

      for (const [source, importInfo] of moduleInfo.imports) {
        if (importInfo.resolvedPath && this.moduleMap.has(importInfo.resolvedPath)) {
          // Internal dependency
          this.dependencyGraph.get(filePath)!.add(importInfo.resolvedPath);
          
          // Build reverse dependency graph
          if (!this.reverseDependencyGraph.has(importInfo.resolvedPath)) {
            this.reverseDependencyGraph.set(importInfo.resolvedPath, new Set());
          }
          this.reverseDependencyGraph.get(importInfo.resolvedPath)!.add(filePath);
        }
      }
    }
  }

  private detectCircularDependencies(): CircularDependency[] {
    const visited = new Set<string>();
    const recursionStack = new Set<string>();
    const circularDeps: CircularDependency[] = [];

    const dfs = (filePath: string, path: string[]): void => {
      if (recursionStack.has(filePath)) {
        // Found a cycle
        const cycleStart = path.indexOf(filePath);
        const cycle = path.slice(cycleStart).concat([filePath]);
        
        circularDeps.push({
          modules: cycle,
          entryPoint: filePath,
          severity: cycle.length > 3 ? 'error' : 'warning',
          suggestedFix: this.suggestCircularDependencyFix(cycle)
        });
        return;
      }

      if (visited.has(filePath)) return;

      visited.add(filePath);
      recursionStack.add(filePath);

      const dependencies = this.dependencyGraph.get(filePath) || new Set();
      for (const dep of dependencies) {
        dfs(dep, [...path, filePath]);
      }

      recursionStack.delete(filePath);
    };

    for (const filePath of this.moduleMap.keys()) {
      if (!visited.has(filePath)) {
        dfs(filePath, []);
      }
    }

    return circularDeps;
  }

  private suggestCircularDependencyFix(cycle: string[]): string {
    if (cycle.length === 2) {
      return 'Consider extracting shared functionality to a separate module';
    } else if (cycle.length === 3) {
      return 'Consider dependency injection or event-driven architecture';
    } else {
      return 'Consider refactoring to reduce coupling between modules';
    }
  }

  // Public methods for relationship analysis

  getDependencyChain(fromModule: string, toModule: string, maxDepth: number = 5): DependencyChain[] {
    const chains: DependencyChain[] = [];
    const visited = new Set<string>();

    const findPaths = (current: string, target: string, path: string[], depth: number): void => {
      if (depth > maxDepth || visited.has(current)) return;
      
      if (current === target && path.length > 0) {
        chains.push({
          modules: [...path, current],
          relationships: [], // Will be populated
          chainType: 'linear',
          depth: path.length,
          strength: this.calculateChainStrength(path),
          description: this.generateChainDescription([...path, current])
        });
        return;
      }

      visited.add(current);
      const dependencies = this.dependencyGraph.get(current) || new Set();
      
      for (const dep of dependencies) {
        findPaths(dep, target, [...path, current], depth + 1);
      }

      visited.delete(current);
    };

    findPaths(fromModule, toModule, [], 0);
    return chains;
  }

  getModuleDependents(modulePath: string): string[] {
    return Array.from(this.reverseDependencyGraph.get(modulePath) || new Set());
  }

  getModuleDependencies(modulePath: string): string[] {
    return Array.from(this.dependencyGraph.get(modulePath) || new Set());
  }

  getTransitiveDependencies(modulePath: string, maxDepth: number = 3): Set<string> {
    const transitive = new Set<string>();
    const visited = new Set<string>();

    const traverse = (current: string, depth: number): void => {
      if (depth > maxDepth || visited.has(current)) return;
      
      visited.add(current);
      const deps = this.dependencyGraph.get(current) || new Set();
      
      for (const dep of deps) {
        transitive.add(dep);
        traverse(dep, depth + 1);
      }
    };

    traverse(modulePath, 0);
    return transitive;
  }

  generateDependencyRelationships(): CodeRelationship[] {
    const relationships: CodeRelationship[] = [];

    for (const [filePath, moduleInfo] of this.moduleMap) {
      for (const [source, importInfo] of moduleInfo.imports) {
        if (importInfo.resolvedPath) {
          const relationship: CodeRelationship = {
            id: `${filePath}:depends_on:${importInfo.resolvedPath}`,
            fromSymbol: `${filePath}:module:1`,
            toSymbol: `${importInfo.resolvedPath}:module:1`,
            type: 'depends_on',
            strength: this.calculateDependencyStrength(importInfo),
            confidence: importInfo.resolvedPath ? 0.9 : 0.6,
            metadata: {
              importType: importInfo.type,
              importedSymbols: importInfo.name.split(', '),
              frequency: importInfo.usageCount,
              sourceLocation: { line: 1, column: 0 }
            }
          };

          relationships.push(relationship);
        }
      }
    }

    return relationships;
  }

  private calculateChainStrength(path: string[]): number {
    let totalStrength = 0;
    
    for (let i = 0; i < path.length - 1; i++) {
      const moduleInfo = this.moduleMap.get(path[i]);
      if (moduleInfo) {
        for (const importInfo of moduleInfo.imports.values()) {
          if (importInfo.resolvedPath === path[i + 1]) {
            totalStrength += this.calculateDependencyStrength(importInfo);
            break;
          }
        }
      }
    }
    
    return path.length > 1 ? totalStrength / (path.length - 1) : 0;
  }

  private calculateDependencyStrength(importInfo: ImportInfo): number {
    let strength = 0.5; // Base strength
    
    // Increase strength based on usage frequency
    if (importInfo.usageCount > 10) strength += 0.3;
    else if (importInfo.usageCount > 5) strength += 0.2;
    else if (importInfo.usageCount > 1) strength += 0.1;
    
    // Increase strength for default imports (usually more important)
    if (importInfo.type === 'default') strength += 0.1;
    
    // Decrease strength for type-only imports
    if (importInfo.isTypeOnly) strength -= 0.2;
    
    return Math.max(0.1, Math.min(1.0, strength));
  }

  private generateChainDescription(modules: string[]): string {
    const moduleNames = modules.map(m => path.basename(m, path.extname(m)));
    
    if (modules.length === 2) {
      return `${moduleNames[0]} depends on ${moduleNames[1]}`;
    } else if (modules.length === 3) {
      return `${moduleNames[0]} → ${moduleNames[1]} → ${moduleNames[2]}`;
    } else {
      return `${moduleNames[0]} → ... → ${moduleNames[modules.length - 1]} (${modules.length - 1} hops)`;
    }
  }

  // Getters for external access
  getModuleMap(): Map<string, ModuleInfo> {
    return this.moduleMap;
  }

  getExternalModules(): Map<string, ModuleInfo> {
    return this.externalModules;
  }

  getDependencyGraph(): Map<string, Set<string>> {
    return this.dependencyGraph;
  }

  getCircularDependencies(): CircularDependency[] {
    return this.detectCircularDependencies();
  }

  // Utility methods for integration
  findModulesImporting(symbol: string): string[] {
    const importingModules: string[] = [];
    
    for (const [filePath, moduleInfo] of this.moduleMap) {
      for (const importInfo of moduleInfo.imports.values()) {
        if (importInfo.name.includes(symbol)) {
          importingModules.push(filePath);
          break;
        }
      }
    }
    
    return importingModules;
  }

  findModulesExporting(symbol: string): string[] {
    const exportingModules: string[] = [];
    
    for (const [filePath, moduleInfo] of this.moduleMap) {
      if (moduleInfo.exports.has(symbol)) {
        exportingModules.push(filePath);
      }
    }
    
    return exportingModules;
  }

  getModuleStats() {
    return {
      totalModules: this.moduleMap.size,
      externalModules: this.externalModules.size,
      totalDependencies: Array.from(this.dependencyGraph.values())
        .reduce((sum, deps) => sum + deps.size, 0),
      circularDependencies: this.detectCircularDependencies().length,
      maxDependencyDepth: this.calculateMaxDependencyDepth()
    };
  }

  private calculateMaxDependencyDepth(): number {
    let maxDepth = 0;
    
    const calculateDepth = (module: string, visited: Set<string>): number => {
      if (visited.has(module)) return 0;
      
      visited.add(module);
      const deps = this.dependencyGraph.get(module) || new Set();
      
      let depth = 0;
      for (const dep of deps) {
        depth = Math.max(depth, calculateDepth(dep, visited));
      }
      
      visited.delete(module);
      return depth + 1;
    };
    
    for (const module of this.moduleMap.keys()) {
      maxDepth = Math.max(maxDepth, calculateDepth(module, new Set()));
    }
    
    return maxDepth;
  }
}