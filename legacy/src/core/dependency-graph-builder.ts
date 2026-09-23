/**
 * Dependency Graph Builder
 * Constructs and maintains a comprehensive dependency graph for cross-file analysis
 */

import { glob } from 'glob';
import { statSync } from 'fs';
import { ASTParser } from './ast-parser.js';
import type {
  DependencyGraph,
  FileNode,
  ImportDeclaration,
  ExportDeclaration,
  SymbolDefinition,
  SymbolUsage
} from '../types/cross-file-analysis.js';

export class DependencyGraphBuilder {
  private projectRoot: string;
  private astParser: ASTParser;
  private graph: DependencyGraph;

  constructor(projectRoot: string) {
    this.projectRoot = projectRoot;
    this.astParser = new ASTParser(projectRoot);
    this.graph = this.initializeGraph();
  }

  /**
   * Build complete dependency graph for the project
   */
  async buildGraph(): Promise<DependencyGraph> {
    console.log('🔍 Building dependency graph...');
    
    // Step 1: Discover all relevant files
    const files = await this.discoverFiles();
    console.log(`📁 Found ${files.length} files to analyze`);

    // Step 2: Parse all files and extract AST information
    await this.parseAllFiles(files);
    console.log(`✅ Parsed ${this.graph.files.size} files successfully`);

    // Step 3: Build import/export relationships
    await this.buildImportExportGraph();
    console.log(`🔗 Built import/export relationships`);

    // Step 4: Extract symbol definitions and usages
    await this.extractSymbolInformation();
    console.log(`🎯 Extracted symbol definitions and usages`);

    // Step 5: Build dependency edges
    this.buildDependencyEdges();
    console.log(`📊 Constructed dependency edges`);

    // Step 6: Calculate reverse dependencies
    this.calculateReverseDependencies();
    console.log(`🔄 Calculated reverse dependencies`);

    return this.graph;
  }

  /**
   * Update graph when files change
   */
  async updateFile(filePath: string): Promise<void> {
    console.log(`🔄 Updating dependency graph for ${filePath}`);

    try {
      // Re-parse the changed file
      const fileNode = await this.astParser.parseFile(filePath);
      this.graph.files.set(filePath, fileNode);

      // Re-extract imports/exports for this file
      if (fileNode.ast) {
        const imports = this.astParser.extractImports(filePath, fileNode.ast);
        const exports = this.astParser.extractExports(fileNode.ast);
        const symbols = this.astParser.extractSymbolDefinitions(filePath, fileNode.ast);
        const usages = this.astParser.extractSymbolUsages(filePath, fileNode.ast);

        this.graph.imports.set(filePath, imports);
        this.graph.exports.set(filePath, exports);
        this.updateSymbolMaps(filePath, symbols, usages);
      }

      // Rebuild dependencies for this file and its dependents
      this.rebuildDependenciesForFile(filePath);
      
    } catch (error) {
      console.error(`Failed to update ${filePath}:`, error);
    }
  }

  /**
   * Get dependency graph
   */
  getGraph(): DependencyGraph {
    return this.graph;
  }

  /**
   * Find all files that depend on a specific file
   */
  findDependents(filePath: string): string[] {
    return this.graph.dependents.get(filePath) || [];
  }

  /**
   * Find all files that a specific file depends on
   */
  findDependencies(filePath: string): string[] {
    return this.graph.dependencies.get(filePath) || [];
  }

  /**
   * Find execution paths between two files
   */
  findPathsBetweenFiles(fromFile: string, toFile: string): string[][] {
    const paths: string[][] = [];
    const visited = new Set<string>();

    const dfs = (currentFile: string, targetFile: string, currentPath: string[]) => {
      if (currentFile === targetFile) {
        paths.push([...currentPath, currentFile]);
        return;
      }

      if (visited.has(currentFile) || currentPath.length > 10) {
        return; // Avoid cycles and infinite recursion
      }

      visited.add(currentFile);
      const dependencies = this.graph.dependencies.get(currentFile) || [];

      for (const dep of dependencies) {
        dfs(dep, targetFile, [...currentPath, currentFile]);
      }

      visited.delete(currentFile);
    };

    dfs(fromFile, toFile, []);
    return paths;
  }

  private initializeGraph(): DependencyGraph {
    return {
      files: new Map(),
      imports: new Map(),
      exports: new Map(),
      symbols: new Map(),
      usages: new Map(),
      dependencies: new Map(),
      dependents: new Map()
    };
  }

  private async discoverFiles(): Promise<string[]> {
    try {
      const patterns = [
        '**/*.ts',
        '**/*.tsx', 
        '**/*.js',
        '**/*.jsx'
      ];

      const allFiles: string[] = [];
      
      for (const pattern of patterns) {
        const matches = glob.sync(pattern, {
          cwd: this.projectRoot,
          absolute: true,
          ignore: [
            'node_modules/**',
            'dist/**',
            'build/**',
            '.next/**',
            'coverage/**',
            '**/*.d.ts', // Skip type definitions
            '**/*.test.*', // Skip test files for now
            '**/*.spec.*'
          ]
        });
        allFiles.push(...matches);
      }

      // Remove duplicates and sort by modification time (newest first)
      const uniqueFiles = [...new Set(allFiles)];
      return uniqueFiles.sort((a, b) => {
        try {
          return statSync(b).mtime.getTime() - statSync(a).mtime.getTime();
        } catch {
          return 0;
        }
      });

    } catch (error) {
      console.error('Error discovering files:', error);
      return [];
    }
  }

  private async parseAllFiles(files: string[]): Promise<void> {
    const batchSize = 10; // Process files in batches to avoid memory issues
    
    for (let i = 0; i < files.length; i += batchSize) {
      const batch = files.slice(i, i + batchSize);
      
      const promises = batch.map(async (file) => {
        try {
          const fileNode = await this.astParser.parseFile(file);
          this.graph.files.set(file, fileNode);
          return fileNode;
        } catch (error) {
          console.warn(`Failed to parse ${file}:`, error);
          return null;
        }
      });

      await Promise.all(promises);
    }
  }

  private async buildImportExportGraph(): Promise<void> {
    for (const [filePath, fileNode] of this.graph.files) {
      if (!fileNode.ast) continue;

      try {
        // Extract imports
        const imports = this.astParser.extractImports(filePath, fileNode.ast);
        this.graph.imports.set(filePath, imports);

        // Extract exports
        const exports = this.astParser.extractExports(fileNode.ast);
        this.graph.exports.set(filePath, exports);

      } catch (error) {
        console.warn(`Failed to extract imports/exports for ${filePath}:`, error);
      }
    }
  }

  private async extractSymbolInformation(): Promise<void> {
    for (const [filePath, fileNode] of this.graph.files) {
      if (!fileNode.ast) continue;

      try {
        // Extract symbol definitions
        const symbols = this.astParser.extractSymbolDefinitions(filePath, fileNode.ast);
        
        // Extract symbol usages
        const usages = this.astParser.extractSymbolUsages(filePath, fileNode.ast);

        this.updateSymbolMaps(filePath, symbols, usages);

      } catch (error) {
        console.warn(`Failed to extract symbols for ${filePath}:`, error);
      }
    }
  }

  private updateSymbolMaps(filePath: string, symbols: SymbolDefinition[], usages: SymbolUsage[]): void {
    // Update symbols map
    for (const symbol of symbols) {
      const existing = this.graph.symbols.get(symbol.name) || [];
      // Remove any existing definitions from this file
      const filtered = existing.filter(s => s.definedIn !== filePath);
      this.graph.symbols.set(symbol.name, [...filtered, symbol]);
    }

    // Update usages map
    for (const usage of usages) {
      const existing = this.graph.usages.get(usage.name) || [];
      // Remove any existing usages from this file
      const filtered = existing.filter(u => u.usedIn !== filePath);
      this.graph.usages.set(usage.name, [...filtered, usage]);
    }
  }

  private buildDependencyEdges(): void {
    // Clear existing dependencies
    this.graph.dependencies.clear();

    for (const [filePath, imports] of this.graph.imports) {
      const dependencies: string[] = [];

      for (const importDecl of imports) {
        if (importDecl.resolvedPath && this.graph.files.has(importDecl.resolvedPath)) {
          dependencies.push(importDecl.resolvedPath);
        }
      }

      if (dependencies.length > 0) {
        this.graph.dependencies.set(filePath, [...new Set(dependencies)]);
      }
    }
  }

  private calculateReverseDependencies(): void {
    this.graph.dependents.clear();

    for (const [filePath, dependencies] of this.graph.dependencies) {
      for (const dependency of dependencies) {
        const existingDependents = this.graph.dependents.get(dependency) || [];
        existingDependents.push(filePath);
        this.graph.dependents.set(dependency, [...new Set(existingDependents)]);
      }
    }
  }

  private rebuildDependenciesForFile(filePath: string): void {
    // Rebuild dependencies for the changed file
    const imports = this.graph.imports.get(filePath) || [];
    const dependencies: string[] = [];

    for (const importDecl of imports) {
      if (importDecl.resolvedPath && this.graph.files.has(importDecl.resolvedPath)) {
        dependencies.push(importDecl.resolvedPath);
      }
    }

    if (dependencies.length > 0) {
      this.graph.dependencies.set(filePath, [...new Set(dependencies)]);
    } else {
      this.graph.dependencies.delete(filePath);
    }

    // Rebuild reverse dependencies
    // Remove this file from all dependents lists
    for (const [file, dependents] of this.graph.dependents) {
      const filtered = dependents.filter(d => d !== filePath);
      if (filtered.length > 0) {
        this.graph.dependents.set(file, filtered);
      } else {
        this.graph.dependents.delete(file);
      }
    }

    // Re-add this file to its dependencies' dependents lists
    for (const dependency of dependencies) {
      const existingDependents = this.graph.dependents.get(dependency) || [];
      existingDependents.push(filePath);
      this.graph.dependents.set(dependency, [...new Set(existingDependents)]);
    }
  }

  /**
   * Get statistics about the dependency graph
   */
  getGraphStats(): {
    totalFiles: number;
    totalImports: number;
    totalExports: number;
    totalSymbols: number;
    totalUsages: number;
    averageDependenciesPerFile: number;
    circularDependencies: string[][];
  } {
    const totalImports = Array.from(this.graph.imports.values())
      .reduce((sum, imports) => sum + imports.length, 0);
    
    const totalExports = Array.from(this.graph.exports.values())
      .reduce((sum, exports) => sum + exports.length, 0);

    const totalSymbols = Array.from(this.graph.symbols.values())
      .reduce((sum, symbols) => sum + symbols.length, 0);

    const totalUsages = Array.from(this.graph.usages.values())
      .reduce((sum, usages) => sum + usages.length, 0);

    const totalDependencies = Array.from(this.graph.dependencies.values())
      .reduce((sum, deps) => sum + deps.length, 0);

    const averageDependenciesPerFile = this.graph.dependencies.size > 0 
      ? totalDependencies / this.graph.dependencies.size 
      : 0;

    const circularDependencies = this.findCircularDependencies();

    return {
      totalFiles: this.graph.files.size,
      totalImports,
      totalExports,
      totalSymbols,
      totalUsages,
      averageDependenciesPerFile: Math.round(averageDependenciesPerFile * 100) / 100,
      circularDependencies
    };
  }

  private findCircularDependencies(): string[][] {
    const cycles: string[][] = [];
    const visited = new Set<string>();
    const recursionStack = new Set<string>();

    const dfs = (filePath: string, currentPath: string[]) => {
      if (recursionStack.has(filePath)) {
        // Found a cycle
        const cycleStart = currentPath.indexOf(filePath);
        if (cycleStart >= 0) {
          cycles.push([...currentPath.slice(cycleStart), filePath]);
        }
        return;
      }

      if (visited.has(filePath)) {
        return;
      }

      visited.add(filePath);
      recursionStack.add(filePath);

      const dependencies = this.graph.dependencies.get(filePath) || [];
      for (const dep of dependencies) {
        dfs(dep, [...currentPath, filePath]);
      }

      recursionStack.delete(filePath);
    };

    for (const filePath of this.graph.files.keys()) {
      if (!visited.has(filePath)) {
        dfs(filePath, []);
      }
    }

    return cycles;
  }
}