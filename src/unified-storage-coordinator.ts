import { PersistentVectorStore } from './persistent-vector-store';
import { PersistentRelationshipStore } from './persistent-relationship-store';
import { RelationshipGraph } from './relationship-types';
import { CodeChunk, ModelInfo } from './types';
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
      console.log('📋 Unified storage coordinator already initialized, skipping...');
      return;
    }

    console.log('🔄 Initializing unified storage coordinator...');
    await Promise.all([
      this.vectorStore.initialize(),
      this.relationshipStore.initialize()
    ]);

    // Check for synchronization issues and auto-sync if needed
    console.log('🔍 Checking storage synchronization...');
    const status = await this.getStorageStatus();
    
    if (!status.synchronized) {
      console.log('⚠️ Storage synchronization issues detected, performing auto-sync...');
      await this.performAutoSync(status);
      console.log('✅ Auto-sync completed');
    } else {
      console.log('✅ Storage layers are synchronized');
    }

    console.log('✅ Unified storage coordinator ready');
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
    // Handle embeddings staleness (both exist but >24h apart)
    else if (status.embeddings.local && status.embeddings.global && 
             embeddingInfo.local.lastModified && embeddingInfo.global.lastModified) {
      const timeDiff = Math.abs(embeddingInfo.local.lastModified.getTime() - embeddingInfo.global.lastModified.getTime());
      const hoursApart = timeDiff / (60 * 60 * 1000);
      
      if (hoursApart > 24) {
        const localIsNewer = embeddingInfo.local.lastModified > embeddingInfo.global.lastModified;
        const olderHours = Math.floor(hoursApart);
        
        if (localIsNewer) {
          issues.push(`Embeddings are ${olderHours} hours apart - local version is newer`);
          actions.push('Auto-syncing newer local embeddings to global storage');
          await this.vectorStore.syncToGlobal();
        } else {
          issues.push(`Embeddings are ${olderHours} hours apart - global version is newer`);
          actions.push('Auto-syncing newer global embeddings to local storage');
          await this.vectorStore.syncToLocal();
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
      console.log('   Issues found:');
      issues.forEach(issue => console.log(`   • ${issue}`));
      console.log('   Actions taken:');
      actions.forEach(action => console.log(`   • ${action}`));
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
        const stats = await fs.stat(relationshipPath).catch(() => null);
        if (stats) {
          relationshipSize = `${(stats.size / 1024).toFixed(2)} KB`;
          
          // Try to read metadata
          const data = JSON.parse(await fs.readFile(relationshipPath, 'utf-8'));
          symbolCount = data.metadata?.totalSymbols || 0;
          relationshipCount = data.metadata?.totalRelationships || 0;
        }
      }
    } catch (error) {
      console.warn('Could not read relationship stats:', error instanceof Error ? error.message : error);
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
    console.log('🔄 Syncing all storage layers...');
    
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
    
    console.log('✅ All storage layers synchronized');
  }

  async clearAll(): Promise<void> {
    console.log('🗑️ Clearing all storage...');
    
    await Promise.all([
      this.vectorStore.clear(),
      this.relationshipStore.clearRelationshipGraph()
    ]);
    
    console.log('✅ All storage cleared');
  }

  async saveAll(chunks: CodeChunk[], relationshipGraph: RelationshipGraph, modelInfo?: ModelInfo): Promise<void> {
    console.log('💾 Saving all data to unified storage...');
    const startTime = Date.now();
    
    // Save both storages in parallel for consistency
    await Promise.all([
      this.vectorStore.savePersistedIndex(modelInfo),
      this.relationshipStore.savePersistedRelationshipGraph(relationshipGraph)
    ]);
    
    const saveTime = Date.now() - startTime;
    console.log(`✅ Unified storage save completed in ${saveTime}ms`);
  }

  async loadAll(): Promise<{
    chunks: CodeChunk[];
    relationshipGraph: RelationshipGraph | null;
    loadedFromCache: boolean;
  }> {
    console.log('📥 Loading from unified storage...');
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
    console.log(`📊 Unified storage load ${status} in ${loadTime}ms`);
    console.log(`   Chunks: ${chunks.length}, Relationships: ${relationshipGraph ? 'loaded' : 'missing'}`);
    
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
    console.log('📊 Unified Storage Report');
    console.log('========================');
    
    const [status, stats, consistency] = await Promise.all([
      this.getStorageStatus(),
      this.getStorageStats(),
      this.validateConsistency()
    ]);
    
    // Storage status
    console.log('\n📁 Storage Status:');
    console.log(`   Embeddings: Local ${status.embeddings.local ? '✅' : '❌'} | Global ${status.embeddings.global ? '✅' : '❌'}`);
    console.log(`   Relationships: Local ${status.relationships.local ? '✅' : '❌'} | Global ${status.relationships.global ? '✅' : '❌'}`);
    console.log(`   Synchronized: ${status.synchronized ? '✅' : '❌'}`);
    
    // Storage statistics
    console.log('\n📈 Storage Statistics:');
    console.log(`   Embeddings: ${stats.embeddings.chunks} chunks across ${stats.embeddings.files} files (${stats.embeddings.size})`);
    console.log(`   Relationships: ${stats.relationships.symbols} symbols, ${stats.relationships.relationships} relationships (${stats.relationships.size})`);
    console.log(`   Total size: ${stats.totalSize}`);
    
    // Consistency check
    console.log('\n🔍 Consistency Check:');
    console.log(`   Status: ${consistency.consistent ? '✅ Consistent' : '⚠️ Issues found'}`);
    
    if (consistency.issues.length > 0) {
      console.log('\n❌ Issues:');
      consistency.issues.forEach(issue => console.log(`   • ${issue}`));
    }
    
    if (consistency.recommendations.length > 0) {
      console.log('\n💡 Recommendations:');
      consistency.recommendations.forEach(rec => console.log(`   • ${rec}`));
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