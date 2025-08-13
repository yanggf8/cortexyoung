import { 
  RelationshipGraph, 
  CodeSymbol, 
  CodeRelationship
} from './relationship-types';
import { CORTEX_SCHEMA_VERSION } from './types';
import * as fs from 'fs/promises';
import * as path from 'path';
import * as crypto from 'crypto';
import * as os from 'os';

interface PersistedRelationshipGraph {
  version: string;
  schemaVersion: string;
  timestamp: number;
  repositoryPath: string;
  symbols: Map<string, CodeSymbol>;
  relationships: Map<string, CodeRelationship>;
  metadata: {
    totalSymbols: number;
    totalRelationships: number;
    lastAnalyzed: number;
    analysisMode: string;
  };
}

export class PersistentRelationshipStore {
  private repositoryPath: string;
  private localGraphPath: string;
  private globalGraphPath: string;
  private metadataPath: string;
  private globalMetadataPath: string;

  constructor(repositoryPath: string, indexDir: string = '.cortex') {
    this.repositoryPath = repositoryPath;
    
    // Local storage (in repo)
    this.localGraphPath = path.join(repositoryPath, indexDir);
    this.metadataPath = path.join(this.localGraphPath, 'relationships.json');
    
    // Global storage (in ~/.claude)
    const repoHash = this.getRepositoryHash(repositoryPath);
    const claudeDir = path.join(os.homedir(), '.claude', 'cortex-embeddings');
    this.globalGraphPath = path.join(claudeDir, repoHash);
    this.globalMetadataPath = path.join(this.globalGraphPath, 'relationships.json');
  }

  private getRepositoryHash(repoPath: string): string {
    const absolutePath = path.resolve(repoPath);
    const hash = crypto.createHash('sha256').update(absolutePath).digest('hex');
    const repoName = path.basename(absolutePath);
    return `${repoName}-${hash.substring(0, 16)}`;
  }

  async initialize(): Promise<void> {
    // Ensure both directories exist
    await fs.mkdir(this.localGraphPath, { recursive: true });
    await fs.mkdir(this.globalGraphPath, { recursive: true });
  }

  async relationshipGraphExists(): Promise<boolean> {
    try {
      await fs.access(this.metadataPath);
      return true;
    } catch {
      return false;
    }
  }

  async globalRelationshipGraphExists(): Promise<boolean> {
    try {
      await fs.access(this.globalMetadataPath);
      return true;
    } catch {
      return false;
    }
  }

  async loadPersistedRelationshipGraph(useGlobal: boolean = false): Promise<RelationshipGraph | null> {
    try {
      const graphPath = useGlobal ? this.globalMetadataPath : this.metadataPath;
      const source = useGlobal ? 'global (~/.claude)' : 'local (.cortex)';
      
      console.log(`🔗 Loading persisted relationship graph from ${source}...`);
      const startTime = Date.now();
      
      const graphData = await fs.readFile(graphPath, 'utf-8');
      const persistedGraph: PersistedRelationshipGraph = JSON.parse(graphData);
      
      // Reconstruct the relationship graph
      const graph: RelationshipGraph = {
        symbols: new Map(Object.entries(persistedGraph.symbols as any)),
        relationships: new Map(Object.entries(persistedGraph.relationships as any)),
        symbolsByFile: new Map(),
        symbolsByType: new Map(),
        relationshipsByType: new Map(),
        outgoingRelationships: new Map(),
        incomingRelationships: new Map()
      };

      // Rebuild indexes
      this.rebuildGraphIndexes(graph);
      
      const loadTime = Date.now() - startTime;
      console.log(`✅ Loaded ${graph.symbols.size} symbols and ${graph.relationships.size} relationships from ${source} in ${loadTime}ms`);
      
      return graph;
    } catch (error) {
      console.warn('⚠️ Failed to load persisted relationship graph:', error instanceof Error ? error.message : error);
      return null;
    }
  }

  async savePersistedRelationshipGraph(graph: RelationshipGraph): Promise<void> {
    try {
      console.log('💾 Saving relationship graph to both local and global storage...');
      const startTime = Date.now();
      
      const persistedGraph: PersistedRelationshipGraph = {
        version: '1.0.0', // Legacy version field
        schemaVersion: CORTEX_SCHEMA_VERSION,
        timestamp: Date.now(),
        repositoryPath: this.repositoryPath,
        symbols: graph.symbols,
        relationships: graph.relationships,
        metadata: {
          totalSymbols: graph.symbols.size,
          totalRelationships: graph.relationships.size,
          lastAnalyzed: Date.now(),
          analysisMode: 'full'
        }
      };
      
      const graphData = JSON.stringify(persistedGraph, this.mapReplacer, 2);
      
      // Save to local storage
      const localTempPath = this.metadataPath + '.tmp';
      await fs.writeFile(localTempPath, graphData);
      await fs.rename(localTempPath, this.metadataPath);
      
      // Save to global storage
      const globalTempPath = this.globalMetadataPath + '.tmp';
      await fs.writeFile(globalTempPath, graphData);
      await fs.rename(globalTempPath, this.globalMetadataPath);
      
      const saveTime = Date.now() - startTime;
      console.log(`✅ Saved relationship graph to both storages in ${saveTime}ms`);
      console.log(`📁 Local: ${this.metadataPath}`);
      console.log(`🌐 Global: ${this.globalMetadataPath}`);
    } catch (error) {
      console.error('❌ Failed to save persisted relationship graph:', error instanceof Error ? error.message : error);
      throw error;
    }
  }

  async syncToGlobal(): Promise<void> {
    try {
      if (await this.relationshipGraphExists()) {
        console.log('🔄 Syncing local relationship graph to global storage...');
        const graphData = await fs.readFile(this.metadataPath, 'utf-8');
        const globalTempPath = this.globalMetadataPath + '.tmp';
        await fs.writeFile(globalTempPath, graphData);
        await fs.rename(globalTempPath, this.globalMetadataPath);
        console.log('✅ Synced relationship graph to global storage');
      }
    } catch (error) {
      console.warn('⚠️ Failed to sync relationship graph to global storage:', error instanceof Error ? error.message : error);
    }
  }

  async syncToLocal(): Promise<void> {
    try {
      if (await this.globalRelationshipGraphExists()) {
        const localExists = await this.relationshipGraphExists();
        let shouldSync = true;
        
        if (localExists) {
          const [localStats, globalStats] = await Promise.all([
            fs.stat(this.metadataPath),
            fs.stat(this.globalMetadataPath)
          ]);
          shouldSync = globalStats.mtime > localStats.mtime;
        }
        
        if (shouldSync) {
          console.log('🔄 Syncing global relationship graph to local storage...');
          const graphData = await fs.readFile(this.globalMetadataPath, 'utf-8');
          const localTempPath = this.metadataPath + '.tmp';
          await fs.writeFile(localTempPath, graphData);
          await fs.rename(localTempPath, this.metadataPath);
          console.log('✅ Synced relationship graph to local storage');
        } else {
          console.log('📋 Local relationship graph is up to date');
        }
      }
    } catch (error) {
      console.warn('⚠️ Failed to sync relationship graph to local storage:', error instanceof Error ? error.message : error);
    }
  }

  async clearRelationshipGraph(): Promise<void> {
    try {
      await fs.rm(this.localGraphPath, { recursive: true, force: true });
      await fs.mkdir(this.localGraphPath, { recursive: true });
    } catch (error) {
      console.warn('⚠️ Failed to clear relationship graph directory:', error instanceof Error ? error.message : error);
    }
  }

  async getStorageInfo(): Promise<{
    local: { exists: boolean; path: string; lastModified?: Date };
    global: { exists: boolean; path: string; lastModified?: Date };
  }> {
    const [localExists, globalExists] = await Promise.all([
      this.relationshipGraphExists(),
      this.globalRelationshipGraphExists()
    ]);

    const info = {
      local: { exists: localExists, path: this.metadataPath },
      global: { exists: globalExists, path: this.globalMetadataPath }
    } as any;

    if (localExists) {
      const localStats = await fs.stat(this.metadataPath);
      info.local.lastModified = localStats.mtime;
    }

    if (globalExists) {
      const globalStats = await fs.stat(this.globalMetadataPath);
      info.global.lastModified = globalStats.mtime;
    }

    return info;
  }

  private rebuildGraphIndexes(graph: RelationshipGraph): void {
    // Clear existing indexes
    graph.symbolsByFile.clear();
    graph.symbolsByType.clear();
    graph.relationshipsByType.clear();
    graph.outgoingRelationships.clear();
    graph.incomingRelationships.clear();

    // Rebuild symbol indexes
    for (const [symbolId, symbol] of graph.symbols) {
      // Index by file
      if (!graph.symbolsByFile.has(symbol.filePath)) {
        graph.symbolsByFile.set(symbol.filePath, new Set());
      }
      graph.symbolsByFile.get(symbol.filePath)!.add(symbolId);

      // Index by type
      if (!graph.symbolsByType.has(symbol.type)) {
        graph.symbolsByType.set(symbol.type, new Set());
      }
      graph.symbolsByType.get(symbol.type)!.add(symbolId);
    }

    // Rebuild relationship indexes
    for (const [relationshipId, relationship] of graph.relationships) {
      // Index by type
      if (!graph.relationshipsByType.has(relationship.type)) {
        graph.relationshipsByType.set(relationship.type, new Set());
      }
      graph.relationshipsByType.get(relationship.type)!.add(relationshipId);

      // Index outgoing relationships
      if (!graph.outgoingRelationships.has(relationship.fromSymbol)) {
        graph.outgoingRelationships.set(relationship.fromSymbol, new Set());
      }
      graph.outgoingRelationships.get(relationship.fromSymbol)!.add(relationshipId);

      // Index incoming relationships
      if (!graph.incomingRelationships.has(relationship.toSymbol)) {
        graph.incomingRelationships.set(relationship.toSymbol, new Set());
      }
      graph.incomingRelationships.get(relationship.toSymbol)!.add(relationshipId);
    }
  }

  // Custom JSON serializer for Maps
  private mapReplacer(key: string, value: any): any {
    if (value instanceof Map) {
      return Object.fromEntries(value);
    }
    return value;
  }

  // Methods needed by redundancy checker
  async listAllRelationships(): Promise<Array<{id: string, data: any}>> {
    try {
      const graph = await this.loadPersistedRelationshipGraph();
      if (!graph) return [];
      
      return Array.from(graph.relationships.entries()).map(([id, relationship]) => ({
        id,
        data: relationship
      }));
    } catch (error) {
      console.warn('Error listing relationships:', error);
      return [];
    }
  }

  async deleteRelationship(id: string): Promise<void> {
    try {
      const graph = await this.loadPersistedRelationshipGraph();
      if (!graph) return;
      
      graph.relationships.delete(id);
      
      // Rebuild indexes after deletion
      this.rebuildGraphIndexes(graph);
      
      // Save the updated graph
      await this.savePersistedRelationshipGraph(graph);
    } catch (error) {
      console.warn('Error deleting relationship:', error);
    }
  }

  async close(): Promise<void> {
    // No-op for now, could be used for cleanup
  }
}