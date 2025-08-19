import { 
  RelationshipGraph, 
  CodeSymbol, 
  CodeRelationship
} from './relationship-types';
import { CORTEX_SCHEMA_VERSION } from './types';
import { StoragePaths, CompressionUtils } from './storage-constants';
import { timestampedLog, warn as timestampedWarn, error as timestampedError } from './logging-utils';
import * as fs from 'fs/promises';

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
    
    // Get all storage paths using centralized utility
    const paths = StoragePaths.getAllPaths(repositoryPath, indexDir);
    
    // Local storage paths
    this.localGraphPath = paths.local.indexPath;
    this.metadataPath = paths.local.relationshipsPath;
    
    // Global storage paths
    this.globalGraphPath = paths.global.indexPath;
    this.globalMetadataPath = paths.global.relationshipsPath;
  }


  async initialize(): Promise<void> {
    // Ensure both directories exist
    await fs.mkdir(this.localGraphPath, { recursive: true });
    await fs.mkdir(this.globalGraphPath, { recursive: true });
  }

  async relationshipGraphExists(): Promise<boolean> {
    return await CompressionUtils.fileExists(this.metadataPath);
  }

  async globalRelationshipGraphExists(): Promise<boolean> {
    return await CompressionUtils.fileExists(this.globalMetadataPath);
  }

  async loadPersistedRelationshipGraph(useGlobal: boolean = false): Promise<RelationshipGraph | null> {
    try {
      const graphPath = useGlobal ? this.globalMetadataPath : this.metadataPath;
      const source = useGlobal ? 'global (~/.claude)' : 'local (.cortex)';
      
      timestampedLog(`🔗 Loading persisted relationship graph from ${source}...`);
      const startTime = Date.now();
      
      const graphData = await CompressionUtils.readFileWithDecompression(graphPath);
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
      timestampedLog(`✅ Loaded ${graph.symbols.size} symbols and ${graph.relationships.size} relationships from ${source} in ${loadTime}ms`);
      
      return graph;
    } catch (error) {
      timestampedWarn(`⚠️ Failed to load persisted relationship graph: ${error instanceof Error ? error.message : error}`);
      return null;
    }
  }

  async savePersistedRelationshipGraph(graph: RelationshipGraph): Promise<void> {
    try {
      timestampedLog('💾 Saving relationship graph to both local and global storage...');
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
      
      // Save to both storages with automatic compression
      await CompressionUtils.writeFileWithCompression(this.metadataPath, graphData);
      await CompressionUtils.writeFileWithCompression(this.globalMetadataPath, graphData);
      
      const saveTime = Date.now() - startTime;
      timestampedLog(`✅ Saved relationship graph to both storages in ${saveTime}ms`);
      timestampedLog(`📁 Local: ${this.metadataPath}`);
      timestampedLog(`🌐 Global: ${this.globalMetadataPath}`);
    } catch (error) {
      timestampedError(`❌ Failed to save persisted relationship graph: ${error instanceof Error ? error.message : error}`);
      throw error;
    }
  }

  async syncToGlobal(): Promise<void> {
    try {
      if (await this.relationshipGraphExists()) {
        timestampedLog('🔄 Syncing local relationship graph to global storage...');
        const graphData = await CompressionUtils.readFileWithDecompression(this.metadataPath);
        await CompressionUtils.writeFileWithCompression(this.globalMetadataPath, graphData);
        timestampedLog('✅ Synced relationship graph to global storage');
      }
    } catch (error) {
      timestampedWarn(`⚠️ Failed to sync relationship graph to global storage: ${error instanceof Error ? error.message : error}`);
    }
  }

  async syncToLocal(): Promise<void> {
    try {
      if (await this.globalRelationshipGraphExists()) {
        const localExists = await this.relationshipGraphExists();
        let shouldSync = true;
        
        if (localExists) {
          // Get the actual file paths (compressed or uncompressed) for stat operations
          const getActualFilePath = async (basePath: string) => {
            const compressedPath = StoragePaths.getCompressedPath(basePath);
            try {
              await fs.access(compressedPath);
              return compressedPath;
            } catch {
              return basePath;
            }
          };
          
          const [localActualPath, globalActualPath] = await Promise.all([
            getActualFilePath(this.metadataPath),
            getActualFilePath(this.globalMetadataPath)
          ]);
          
          const [localStats, globalStats] = await Promise.all([
            fs.stat(localActualPath),
            fs.stat(globalActualPath)
          ]);
          shouldSync = globalStats.mtime > localStats.mtime;
        }
        
        if (shouldSync) {
          timestampedLog('🔄 Syncing global relationship graph to local storage...');
          const graphData = await CompressionUtils.readFileWithDecompression(this.globalMetadataPath);
          await CompressionUtils.writeFileWithCompression(this.metadataPath, graphData);
          timestampedLog('✅ Synced relationship graph to local storage');
        } else {
          timestampedLog('📋 Local relationship graph is up to date');
        }
      }
    } catch (error) {
      timestampedWarn(`⚠️ Failed to sync relationship graph to local storage: ${error instanceof Error ? error.message : error}`);
    }
  }

  async clearRelationshipGraph(): Promise<void> {
    try {
      await fs.rm(this.localGraphPath, { recursive: true, force: true });
      await fs.mkdir(this.localGraphPath, { recursive: true });
    } catch (error) {
      timestampedWarn(`⚠️ Failed to clear relationship graph directory: ${error instanceof Error ? error.message : error}`);
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

    // Get actual file paths for stat operations (compressed or uncompressed)
    const getActualFilePathForStat = async (basePath: string): Promise<string> => {
      const compressedPath = StoragePaths.getCompressedPath(basePath);
      try {
        await fs.access(compressedPath);
        return compressedPath;
      } catch {
        return basePath;
      }
    };

    if (localExists) {
      const localActualPath = await getActualFilePathForStat(this.metadataPath);
      const localStats = await fs.stat(localActualPath);
      info.local.lastModified = localStats.mtime;
    }

    if (globalExists) {
      const globalActualPath = await getActualFilePathForStat(this.globalMetadataPath);
      const globalStats = await fs.stat(globalActualPath);
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
      timestampedWarn(`Error listing relationships: ${error}`);
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
      timestampedWarn(`Error deleting relationship: ${error}`);
    }
  }

  async close(): Promise<void> {
    // No-op for now, could be used for cleanup
  }
}