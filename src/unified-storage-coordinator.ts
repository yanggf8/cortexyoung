import { PersistentVectorStore } from './persistent-vector-store';
import { PersistentRelationshipStore } from './persistent-relationship-store';
import { RelationshipGraph } from './relationship-types';
import { CodeChunk, ModelInfo } from './types';
import { log } from './logging-utils';
import { StoragePaths, CompressionUtils } from './storage-constants';
import * as fs from 'fs/promises';
import * as path from 'path';

interface StorageStatus {
  embeddings: {
    local: boolean;
    global: boolean;
    lastModified?: Date;
  };
  relationships: {
    local: boolean;
    global: boolean;
    lastModified?: Date;
  };
  synchronized: boolean;
}

interface StorageStats {
  embeddings: {
    chunks: number;
    files: number;
    size: string;
  };
  relationships: {
    symbols: number;
    relationships: number;
    size: string;
  };
  totalSize: string;
}

export class UnifiedStorageCoordinator {
  private vectorStore: PersistentVectorStore;
  private relationshipStore: PersistentRelationshipStore;
  private repositoryPath: string;

  constructor(repositoryPath: string, indexDir: string = '.cortex') {
    this.repositoryPath = repositoryPath;
    this.vectorStore = new PersistentVectorStore(repositoryPath, indexDir);
    this.relationshipStore = new PersistentRelationshipStore(repositoryPath, indexDir);
  }

  private initialized: boolean = false;

  async initialize(): Promise<void> {
    // Prevent duplicate initialization
    if (this.initialized) {
      log('[StorageCoord] Already initialized, skipping');
      return;
    }

    log('[StorageCoord] Initializing unified storage coordinator');
    await Promise.all([
      this.vectorStore.initialize(),
      this.relationshipStore.initialize()
    ]);

    // Check for synchronization issues and auto-sync if needed
    log('[StorageCoord] Checking storage synchronization');
    const status = await this.getStorageStatus();
    
    if (!status.synchronized) {
      log('[StorageCoord] Storage sync issues detected, performing auto-sync');
      await this.performAutoSync(status);
      log('[StorageCoord] Auto-sync completed');
    } else {
      log('[StorageCoord] Storage layers are synchronized');
    }

    log('[StorageCoord] Unified storage coordinator ready');
    this.initialized = true;
  }

  async getStorageStatus(): Promise<StorageStatus> {
    const [embeddingInfo, relationshipInfo] = await Promise.all([
      this.vectorStore.getStorageInfo(),
      this.relationshipStore.getStorageInfo()
    ]);

    const synchronized = this.checkSynchronization(embeddingInfo, relationshipInfo);

    return {
      embeddings: {
        local: embeddingInfo.local.exists,
        global: embeddingInfo.global.exists,
        lastModified: embeddingInfo.local.lastModified || embeddingInfo.global.lastModified
      },
      relationships: {
        local: relationshipInfo.local.exists,
        global: relationshipInfo.global.exists,
        lastModified: relationshipInfo.local.lastModified || relationshipInfo.global.lastModified
      },
      synchronized
    };
  }

  private checkSynchronization(embeddingInfo: any, relationshipInfo: any): boolean {
    // Check if both storages exist and are reasonably synchronized
    const embeddingsExist = embeddingInfo.local.exists || embeddingInfo.global.exists;
    const relationshipsExist = relationshipInfo.local.exists || relationshipInfo.global.exists;

    if (!embeddingsExist || !relationshipsExist) {
      return false; // Not synchronized if either is missing completely
    }

    // Check if both embeddings and relationships exist in at least one matching location
    const localPairExists = embeddingInfo.local.exists && relationshipInfo.local.exists;
    const globalPairExists = embeddingInfo.global.exists && relationshipInfo.global.exists;
    
    if (!localPairExists && !globalPairExists) {
      return false; // Not synchronized if no complete pair exists in same location
    }

    // Check timestamp alignment (within 1 hour tolerance) for matching pairs
    let synchronized = false;
    
    if (localPairExists) {
      const localEmbeddingTime = embeddingInfo.local.lastModified;
      const localRelationshipTime = relationshipInfo.local.lastModified;
      if (localEmbeddingTime && localRelationshipTime) {
        const timeDiff = Math.abs(localEmbeddingTime.getTime() - localRelationshipTime.getTime());
        synchronized = synchronized || timeDiff < 60 * 60 * 1000;
      } else {
        synchronized = true; // Assume synchronized if we can't determine timestamps
      }
    }
    
    if (globalPairExists && !synchronized) {
      const globalEmbeddingTime = embeddingInfo.global.lastModified;
      const globalRelationshipTime = relationshipInfo.global.lastModified;
      if (globalEmbeddingTime && globalRelationshipTime) {
        const timeDiff = Math.abs(globalEmbeddingTime.getTime() - globalRelationshipTime.getTime());
        synchronized = synchronized || timeDiff < 60 * 60 * 1000;
      } else {
        synchronized = true; // Assume synchronized if we can't determine timestamps
      }
    }

    return synchronized;
  }

  private async performAutoSync(status: StorageStatus): Promise<void> {
    const issues: string[] = [];
    const actions: string[] = [];

    // Get detailed storage info for staleness detection
    const [embeddingInfo, relationshipInfo] = await Promise.all([
      this.vectorStore.getStorageInfo(),
      this.relationshipStore.getStorageInfo()
    ]);

    // Handle embeddings synchronization (missing cases)
    if (status.embeddings.local && !status.embeddings.global) {
      issues.push('Local embeddings exist but global embeddings are missing');
      actions.push('Syncing local embeddings to global storage');
      await this.vectorStore.syncToGlobal();
    } else if (!status.embeddings.local && status.embeddings.global) {
      issues.push('Global embeddings exist but local embeddings are missing');
      actions.push('Syncing global embeddings to local storage');
      await this.vectorStore.syncToLocal();
    }
    // Handle embeddings based on commit validity and timestamp
    else if (status.embeddings.local && status.embeddings.global) {
      const localCommitValid = embeddingInfo.local.commitValid ?? false;
      const globalCommitValid = embeddingInfo.global.commitValid ?? false;
      const localChunks = embeddingInfo.local.chunks ?? 0;
      const globalChunks = embeddingInfo.global.chunks ?? 0;
      
      log(`[StorageCompare] Local chunks=${localChunks} modified=${embeddingInfo.local.lastModified?.toISOString()} path=${embeddingInfo.local.path}`);
      log(`[StorageCompare] Global chunks=${globalChunks} modified=${embeddingInfo.global.lastModified?.toISOString()} path=${embeddingInfo.global.path}`);
      log(`[StorageCompare] Local commit_valid=${localCommitValid} commit=${embeddingInfo.local.commitHash}`);
      log(`[StorageCompare] Global commit_valid=${globalCommitValid} commit=${embeddingInfo.global.commitHash}`);
      
      // Priority: Commit validity > Chunk count > Timestamp
      if (localCommitValid && !globalCommitValid) {
        issues.push('Local embeddings match current commit, global embeddings are stale');
        actions.push('Winner=local reason=commit_match loading=local');
        log(`[StorageCompare] Winner=local reason=commit_match loading=local`);
        await this.vectorStore.syncToGlobal();
      } else if (!localCommitValid && globalCommitValid) {
        issues.push('Global embeddings match current commit, local embeddings are stale');
        actions.push('Winner=global reason=commit_match loading=global');
        log(`[StorageCompare] Winner=global reason=commit_match loading=global`);
        await this.vectorStore.syncToLocal();
      } else if (localCommitValid && globalCommitValid) {
        // Both match current commit - use timestamp to determine newer working changes
        if (embeddingInfo.local.lastModified && embeddingInfo.global.lastModified) {
          const localIsNewer = embeddingInfo.local.lastModified > embeddingInfo.global.lastModified;
          if (localIsNewer) {
            issues.push('Both match current commit - local is newer');
            actions.push('Winner=local reason=newer loading=local');
            log(`[StorageCompare] Winner=local reason=newer loading=local`);
            await this.vectorStore.syncToGlobal();
          } else {
            issues.push('Both match current commit - global is newer');
            actions.push('Winner=global reason=newer loading=global');
            log(`[StorageCompare] Winner=global reason=newer loading=global`);
            await this.vectorStore.syncToLocal();
          }
        } else {
          // Fallback: prefer global if no timestamps available
          issues.push('Both match current commit - no timestamps, defaulting to global');
          actions.push('Winner=global reason=no_timestamps loading=global');
          log(`[StorageCompare] Winner=global reason=no_timestamps loading=global`);
          await this.vectorStore.syncToLocal();
        }
      } else {
        // Neither matches current commit - use larger dataset or newer timestamp
        if (localChunks > globalChunks) {
          issues.push('Neither matches current commit - local has more chunks (will need reindexing)');
          actions.push('Winner=local reason=more_chunks_stale loading=local');
          log(`[StorageCompare] Winner=local reason=more_chunks_stale loading=local`);
          await this.vectorStore.syncToGlobal();
        } else if (globalChunks > localChunks) {
          issues.push('Neither matches current commit - global has more chunks (will need reindexing)');
          actions.push('Winner=global reason=more_chunks_stale loading=global');
          log(`[StorageCompare] Winner=global reason=more_chunks_stale loading=global`);
          await this.vectorStore.syncToLocal();
        } else if (embeddingInfo.local.lastModified && embeddingInfo.global.lastModified) {
          const timeDiff = Math.abs(embeddingInfo.local.lastModified.getTime() - embeddingInfo.global.lastModified.getTime());
          const hoursApart = timeDiff / (60 * 60 * 1000);
          
          if (hoursApart > 24) {
            const localIsNewer = embeddingInfo.local.lastModified > embeddingInfo.global.lastModified;
            const olderHours = Math.floor(hoursApart);
            
            if (localIsNewer) {
              issues.push(`Neither matches current commit - local is ${olderHours}h newer (will need reindexing)`);
              actions.push('Winner=local reason=newer_stale loading=local');
              log(`[StorageCompare] Winner=local reason=newer_stale loading=local`);
              await this.vectorStore.syncToGlobal();
            } else {
              issues.push(`Neither matches current commit - global is ${olderHours}h newer (will need reindexing)`);
              actions.push('Winner=global reason=newer_stale loading=global');
              log(`[StorageCompare] Winner=global reason=newer_stale loading=global`);
              await this.vectorStore.syncToLocal();
            }
          }
        }
      }
    }

    // Handle relationships synchronization (missing cases)
    if (status.relationships.local && !status.relationships.global) {
      issues.push('Local relationships exist but global relationships are missing');
      actions.push('Syncing local relationships to global storage');
      await this.relationshipStore.syncToGlobal();
    } else if (!status.relationships.local && status.relationships.global) {
      issues.push('Global relationships exist but local relationships are missing');
      actions.push('Syncing global relationships to local storage');
      await this.relationshipStore.syncToLocal();
    }
    // Handle relationships staleness (both exist but >24h apart)
    else if (status.relationships.local && status.relationships.global && 
             relationshipInfo.local.lastModified && relationshipInfo.global.lastModified) {
      const timeDiff = Math.abs(relationshipInfo.local.lastModified.getTime() - relationshipInfo.global.lastModified.getTime());
      const hoursApart = timeDiff / (60 * 60 * 1000);
      
      if (hoursApart > 24) {
        const localIsNewer = relationshipInfo.local.lastModified > relationshipInfo.global.lastModified;
        const olderHours = Math.floor(hoursApart);
        
        if (localIsNewer) {
          issues.push(`Relationships are ${olderHours} hours apart - local version is newer`);
          actions.push('Auto-syncing newer local relationships to global storage');
          await this.relationshipStore.syncToGlobal();
        } else {
          issues.push(`Relationships are ${olderHours} hours apart - global version is newer`);
          actions.push('Auto-syncing newer global relationships to local storage');
          await this.relationshipStore.syncToLocal();
        }
      }
    }

    // Handle missing relationships when embeddings exist
    if ((status.embeddings.local || status.embeddings.global) && 
        (!status.relationships.local && !status.relationships.global)) {
      issues.push('Embeddings exist but relationships are completely missing');
      actions.push('Relationships will be regenerated during startup');
    }

    // Log what was found and what actions were taken
    if (issues.length > 0) {
      log('[StorageCoord] Issues found:');
      issues.forEach(issue => log(`[StorageCoord] Issue: ${issue}`));
      log('[StorageCoord] Actions taken:');
      actions.forEach(action => log(`[StorageCoord] Action: ${action}`));
    }
  }

  async getStorageStats(): Promise<StorageStats> {
    const [embeddingStats, embeddingInfo, relationshipInfo] = await Promise.all([
      this.vectorStore.getStats(),
      this.vectorStore.getStorageInfo(),
      this.relationshipStore.getStorageInfo()
    ]);

    // Calculate relationship storage size
    let relationshipSize = '0 KB';
    let symbolCount = 0;
    let relationshipCount = 0;

    try {
      const relationshipPath = relationshipInfo.local.exists ? 
        relationshipInfo.local.path : relationshipInfo.global.path;
      
      if (relationshipPath) {
        // Get the actual file path (compressed or uncompressed) for stat operations
        const getActualFilePath = async (basePath: string): Promise<string | null> => {
          const compressedPath = StoragePaths.getCompressedPath(basePath);
          try {
            await fs.access(compressedPath);
            return compressedPath;
          } catch {
            try {
              await fs.access(basePath);
              return basePath;
            } catch {
              return null;
            }
          }
        };

        const actualPath = await getActualFilePath(relationshipPath);
        if (actualPath) {
          const stats = await fs.stat(actualPath).catch(() => null);
          if (stats) {
            relationshipSize = `${(stats.size / 1024).toFixed(2)} KB`;
            
            // Try to read metadata using compression-aware reader
            try {
              const data = JSON.parse(await CompressionUtils.readFileWithDecompression(relationshipPath));
              symbolCount = data.metadata?.totalSymbols || 0;
              relationshipCount = data.metadata?.totalRelationships || 0;
            } catch (error) {
              // Ignore metadata read errors
            }
          }
        }
      }
    } catch (error) {
      log(`[StorageCoord] Could not read relationship stats error=${error instanceof Error ? error.message : error}`);
    }

    // Calculate total size
    const embeddingSizeNum = parseFloat(embeddingStats.indexSize.replace(' MB', '')) * 1024;
    const relationshipSizeNum = parseFloat(relationshipSize.replace(' KB', ''));
    const totalSizeKB = embeddingSizeNum + relationshipSizeNum;
    const totalSize = totalSizeKB > 1024 ? 
      `${(totalSizeKB / 1024).toFixed(2)} MB` : 
      `${totalSizeKB.toFixed(2)} KB`;

    return {
      embeddings: {
        chunks: embeddingStats.total_chunks,
        files: embeddingStats.totalFiles,
        size: embeddingStats.indexSize
      },
      relationships: {
        symbols: symbolCount,
        relationships: relationshipCount,
        size: relationshipSize
      },
      totalSize
    };
  }

  async syncAll(): Promise<void> {
    log('[StorageCoord] Syncing all storage layers');
    
    const status = await this.getStorageStatus();
    
    // Sync embeddings
    if (status.embeddings.local && !status.embeddings.global) {
      await this.vectorStore.syncToGlobal();
    } else if (!status.embeddings.local && status.embeddings.global) {
      await this.vectorStore.syncToLocal();
    }
    
    // Sync relationships
    if (status.relationships.local && !status.relationships.global) {
      await this.relationshipStore.syncToGlobal();
    } else if (!status.relationships.local && status.relationships.global) {
      await this.relationshipStore.syncToLocal();
    }
    
    log('[StorageCoord] All storage layers synchronized');
  }

  async clearAll(): Promise<void> {
    log('[StorageCoord] Clearing all storage');
    
    await Promise.all([
      this.vectorStore.clear(),
      this.relationshipStore.clearRelationshipGraph()
    ]);
    
    log('[StorageCoord] All storage cleared');
  }

  async saveAll(chunks: CodeChunk[], relationshipGraph: RelationshipGraph, modelInfo?: ModelInfo): Promise<void> {
    log('[StorageCoord] Saving all data to unified storage');
    const startTime = Date.now();
    
    // Save both storages in parallel for consistency
    await Promise.all([
      this.vectorStore.savePersistedIndex(modelInfo),
      this.relationshipStore.savePersistedRelationshipGraph(relationshipGraph)
    ]);
    
    const saveTime = Date.now() - startTime;
    log(`[StorageCoord] Unified storage save completed duration=${saveTime}ms`);
  }

  async loadAll(): Promise<{
    chunks: CodeChunk[];
    relationshipGraph: RelationshipGraph | null;
    loadedFromCache: boolean;
  }> {
    log('[StorageCoord] Loading from unified storage');
    const startTime = Date.now();
    
    // Try to load both storages in parallel
    const [embeddingsLoaded, relationshipGraph] = await Promise.all([
      this.vectorStore.loadPersistedIndex().catch(() => false),
      this.relationshipStore.loadPersistedRelationshipGraph().catch(() => null)
    ]);
    
    const chunks = embeddingsLoaded ? this.vectorStore.getAllChunks() : [];
    const loadedFromCache = embeddingsLoaded && relationshipGraph !== null;
    
    const loadTime = Date.now() - startTime;
    const status = loadedFromCache ? 'from cache' : 'partially from cache';
    log(`[StorageCoord] Unified storage load status=${status} duration=${loadTime}ms`);
    log(`[StorageCoord] Chunks loaded=${chunks.length} relationships=${relationshipGraph ? 'loaded' : 'missing'}`);
    
    return {
      chunks,
      relationshipGraph,
      loadedFromCache
    };
  }

  async validateConsistency(): Promise<{
    consistent: boolean;
    issues: string[];
    recommendations: string[];
  }> {
    const issues: string[] = [];
    const recommendations: string[] = [];
    
    const status = await this.getStorageStatus();
    
    // Check if both storage types exist
    if (status.embeddings.local && !status.relationships.local) {
      issues.push('Local embeddings exist but local relationships are missing');
      recommendations.push('Auto-sync will handle this during server startup');
    }
    
    if (status.embeddings.global && !status.relationships.global) {
      issues.push('Global embeddings exist but global relationships are missing');
      recommendations.push('Auto-sync will handle this during server startup');
    }
    
    // Check synchronization
    if (!status.synchronized) {
      issues.push('Embeddings and relationships are not synchronized');
      recommendations.push('Auto-sync will fix this during initialization');
    }
    
    // Check for staleness - embeddings
    if (status.embeddings.local && status.embeddings.global) {
      const embeddingInfo = await this.vectorStore.getStorageInfo();
      if (embeddingInfo.local.lastModified && embeddingInfo.global.lastModified &&
          Math.abs(embeddingInfo.local.lastModified.getTime() - embeddingInfo.global.lastModified.getTime()) > 24 * 60 * 60 * 1000) {
        issues.push('Local and global embeddings are more than 24 hours apart');
        recommendations.push('Auto-sync handles this automatically during server startup');
      }
    }
    
    // Check for staleness - relationships
    if (status.relationships.local && status.relationships.global) {
      const relationshipInfo = await this.relationshipStore.getStorageInfo();
      if (relationshipInfo.local.lastModified && relationshipInfo.global.lastModified &&
          Math.abs(relationshipInfo.local.lastModified.getTime() - relationshipInfo.global.lastModified.getTime()) > 24 * 60 * 60 * 1000) {
        issues.push('Local and global relationships are more than 24 hours apart');
        recommendations.push('Auto-sync handles this automatically during server startup');
      }
    }
    
    return {
      consistent: issues.length === 0,
      issues,
      recommendations
    };
  }

  async printStorageReport(): Promise<void> {
    log('[StorageCoord] Unified Storage Report');
    log('[StorageCoord] ========================');
    
    const [status, stats, consistency] = await Promise.all([
      this.getStorageStatus(),
      this.getStorageStats(),
      this.validateConsistency()
    ]);
    
    // Storage status
    log('[StorageCoord] Storage Status:');
    log(`[StorageCoord] Embeddings local=${status.embeddings.local} global=${status.embeddings.global}`);
    log(`[StorageCoord] Relationships local=${status.relationships.local} global=${status.relationships.global}`);
    log(`[StorageCoord] Synchronized=${status.synchronized}`);
    
    // Storage statistics
    log('[StorageCoord] Storage Statistics:');
    log(`[StorageCoord] Embeddings chunks=${stats.embeddings.chunks} files=${stats.embeddings.files} size=${stats.embeddings.size}`);
    log(`[StorageCoord] Relationships symbols=${stats.relationships.symbols} relationships=${stats.relationships.relationships} size=${stats.relationships.size}`);
    log(`[StorageCoord] Total size=${stats.totalSize}`);
    
    // Consistency check
    log('[StorageCoord] Consistency Check:');
    log(`[StorageCoord] Consistency status=${consistency.consistent ? 'consistent' : 'issues_found'}`);
    
    if (consistency.issues.length > 0) {
      log('[StorageCoord] Issues:');
      consistency.issues.forEach(issue => log(`[StorageCoord] Issue: ${issue}`));
    }
    
    if (consistency.recommendations.length > 0) {
      log('[StorageCoord] Recommendations:');
      consistency.recommendations.forEach(rec => log(`[StorageCoord] Recommendation: ${rec}`));
    }
  }

  // Getters for component access
  getVectorStore(): PersistentVectorStore {
    return this.vectorStore;
  }

  getRelationshipStore(): PersistentRelationshipStore {
    return this.relationshipStore;
  }
}