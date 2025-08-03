import * as fs from 'fs/promises';
import * as path from 'path';
import * as crypto from 'crypto';
import { CodeChunk } from './types';
import { PersistentVectorStore } from './persistent-vector-store';
import { GitScanner } from './git-scanner';

export interface HealthIssue {
  type: 'corruption' | 'staleness' | 'performance' | 'missing_data';
  severity: 'critical' | 'warning' | 'info';
  message: string;
  details?: any;
  suggestedAction: 'full_rebuild' | 'incremental_update' | 'investigate' | 'none';
}

export interface HealthReport {
  overall: 'healthy' | 'degraded' | 'critical';
  issues: HealthIssue[];
  stats: {
    totalChunks: number;
    totalFiles: number;
    lastIndexed: Date | null;
    indexAge: number; // milliseconds
    embeddingModel: string;
    schemaVersion: string;
  };
  recommendations: string[];
}

interface RepositorySnapshot {
  currentBranch: string;
  latestCommit: string;
  dependencyHashes: Map<string, string>;
  buildConfigHashes: Map<string, string>;
}

export class IndexHealthChecker {
  private vectorStore: PersistentVectorStore;
  private gitScanner: GitScanner;
  private repositoryPath: string;

  constructor(repositoryPath: string, vectorStore: PersistentVectorStore) {
    this.repositoryPath = repositoryPath;
    this.vectorStore = vectorStore;
    this.gitScanner = new GitScanner(repositoryPath);
  }

  async performHealthCheck(): Promise<HealthReport> {
    const issues: HealthIssue[] = [];
    
    try {
      // Initialize vector store
      await this.vectorStore.initialize();
      
      // Check if index exists
      const indexExists = await this.vectorStore.indexExists();
      if (!indexExists) {
        return {
          overall: 'critical',
          issues: [{
            type: 'missing_data',
            severity: 'critical',
            message: 'No embedding index found',
            suggestedAction: 'full_rebuild'
          }],
          stats: {
            totalChunks: 0,
            totalFiles: 0,
            lastIndexed: null,
            indexAge: 0,
            embeddingModel: 'unknown',
            schemaVersion: 'unknown'
          },
          recommendations: ['Run full indexing to create initial embeddings']
        };
      }

      // Gather health checks
      const corruptionIssues = await this.checkCorruption();
      const stalenessIssues = await this.checkStaleness();
      const performanceIssues = await this.checkPerformance();
      
      issues.push(...corruptionIssues, ...stalenessIssues, ...performanceIssues);

      // Get current stats
      const stats = await this.getIndexStats();
      
      // Determine overall health
      const overall = this.determineOverallHealth(issues);
      
      // Generate recommendations
      const recommendations = this.generateRecommendations(issues);

      return {
        overall,
        issues,
        stats,
        recommendations
      };
      
    } catch (error) {
      return {
        overall: 'critical',
        issues: [{
          type: 'corruption',
          severity: 'critical',
          message: `Health check failed: ${error instanceof Error ? error.message : 'Unknown error'}`,
          suggestedAction: 'full_rebuild'
        }],
        stats: {
          totalChunks: 0,
          totalFiles: 0,
          lastIndexed: null,
          indexAge: 0,
          embeddingModel: 'unknown',
          schemaVersion: 'unknown'
        },
        recommendations: ['Run full rebuild due to health check failure']
      };
    }
  }

  private async checkCorruption(): Promise<HealthIssue[]> {
    const issues: HealthIssue[] = [];

    try {
      // Check embedding dimension consistency
      const chunks = await this.vectorStore.getAllChunks();
      if (chunks.length > 0) {
        const expectedDimension = chunks[0].embedding?.length || 384;
        const invalidEmbeddings = chunks.filter(chunk => 
          !chunk.embedding || 
          chunk.embedding.length !== expectedDimension ||
          chunk.embedding.some(val => isNaN(val) || !isFinite(val))
        );

        if (invalidEmbeddings.length > 0) {
          const percentage = (invalidEmbeddings.length / chunks.length) * 100;
          issues.push({
            type: 'corruption',
            severity: percentage > 10 ? 'critical' : 'warning',
            message: `Found ${invalidEmbeddings.length} chunks with invalid embeddings (${percentage.toFixed(1)}%)`,
            details: { 
              invalidCount: invalidEmbeddings.length, 
              totalCount: chunks.length,
              expectedDimension 
            },
            suggestedAction: percentage > 10 ? 'full_rebuild' : 'incremental_update'
          });
        }

        // Check for duplicate chunk IDs
        const chunkIds = chunks.map(c => c.chunk_id);
        const uniqueIds = new Set(chunkIds);
        if (chunkIds.length !== uniqueIds.size) {
          issues.push({
            type: 'corruption',
            severity: 'critical',
            message: 'Found duplicate chunk IDs in index',
            details: { 
              totalChunks: chunkIds.length, 
              uniqueChunks: uniqueIds.size 
            },
            suggestedAction: 'full_rebuild'
          });
        }

        // Check for orphaned chunks (files that no longer exist)
        const filePaths = new Set(chunks.map(c => c.file_path));
        const orphanedFiles: string[] = [];
        
        for (const filePath of filePaths) {
          try {
            const fullPath = path.join(this.repositoryPath, filePath);
            await fs.access(fullPath);
          } catch {
            orphanedFiles.push(filePath);
          }
        }

        if (orphanedFiles.length > 0) {
          const orphanedChunks = chunks.filter(c => orphanedFiles.includes(c.file_path));
          const percentage = (orphanedChunks.length / chunks.length) * 100;
          
          issues.push({
            type: 'corruption',
            severity: percentage > 25 ? 'critical' : 'warning',
            message: `Found ${orphanedChunks.length} chunks for deleted files (${percentage.toFixed(1)}%)`,
            details: { 
              orphanedFiles: orphanedFiles.slice(0, 10), // Show first 10
              totalOrphaned: orphanedFiles.length 
            },
            suggestedAction: percentage > 25 ? 'full_rebuild' : 'incremental_update'
          });
        }
      }

    } catch (error) {
      issues.push({
        type: 'corruption',
        severity: 'critical',
        message: `Corruption check failed: ${error instanceof Error ? error.message : 'Unknown error'}`,
        suggestedAction: 'full_rebuild'
      });
    }

    return issues;
  }

  private async checkStaleness(): Promise<HealthIssue[]> {
    const issues: HealthIssue[] = [];

    try {
      // Check git branch changes
      const currentSnapshot = await this.getCurrentRepositorySnapshot();
      const indexedSnapshot = await this.getIndexedRepositorySnapshot();

      if (indexedSnapshot) {
        // Check branch changes
        if (currentSnapshot.currentBranch !== indexedSnapshot.currentBranch) {
          issues.push({
            type: 'staleness',
            severity: 'warning',
            message: `Branch changed from '${indexedSnapshot.currentBranch}' to '${currentSnapshot.currentBranch}'`,
            details: { 
              oldBranch: indexedSnapshot.currentBranch,
              newBranch: currentSnapshot.currentBranch 
            },
            suggestedAction: 'incremental_update'
          });
        }

        // Check major commit divergence
        if (currentSnapshot.latestCommit !== indexedSnapshot.latestCommit) {
          const commitsBehind = await this.getCommitDistance(indexedSnapshot.latestCommit, currentSnapshot.latestCommit);
          if (commitsBehind > 20) {
            issues.push({
              type: 'staleness',
              severity: 'warning',
              message: `Index is ${commitsBehind} commits behind current HEAD`,
              details: { 
                commitsBehind,
                oldCommit: indexedSnapshot.latestCommit.substring(0, 8),
                newCommit: currentSnapshot.latestCommit.substring(0, 8)
              },
              suggestedAction: 'incremental_update'
            });
          }
        }

        // Check dependency changes
        const changedDeps = this.findChangedDependencies(currentSnapshot.dependencyHashes, indexedSnapshot.dependencyHashes);
        if (changedDeps.length > 0) {
          issues.push({
            type: 'staleness',
            severity: 'info',
            message: `Dependency files changed: ${changedDeps.join(', ')}`,
            details: { changedFiles: changedDeps },
            suggestedAction: 'incremental_update'
          });
        }

        // Check build config changes
        const changedConfigs = this.findChangedDependencies(currentSnapshot.buildConfigHashes, indexedSnapshot.buildConfigHashes);
        if (changedConfigs.length > 0) {
          issues.push({
            type: 'staleness',
            severity: 'warning',
            message: `Build configuration changed: ${changedConfigs.join(', ')}`,
            details: { changedFiles: changedConfigs },
            suggestedAction: 'full_rebuild'
          });
        }
      }

      // Check index age
      const stats = await this.getIndexStats();
      const maxAge = 7 * 24 * 60 * 60 * 1000; // 7 days
      if (stats.indexAge > maxAge) {
        const days = Math.floor(stats.indexAge / (24 * 60 * 60 * 1000));
        issues.push({
          type: 'staleness',
          severity: 'info',
          message: `Index is ${days} days old`,
          details: { ageInDays: days },
          suggestedAction: 'incremental_update'
        });
      }

    } catch (error) {
      issues.push({
        type: 'staleness',
        severity: 'warning',
        message: `Staleness check failed: ${error instanceof Error ? error.message : 'Unknown error'}`,
        suggestedAction: 'investigate'
      });
    }

    return issues;
  }

  private async checkPerformance(): Promise<HealthIssue[]> {
    const issues: HealthIssue[] = [];

    try {
      // Check file coverage
      const scanResult = await this.gitScanner.scanRepository('full');
      const chunks = await this.vectorStore.getAllChunks();
      const indexedFiles = new Set(chunks.map(c => c.file_path));
      const totalFiles = scanResult.files.length;
      const coverage = (indexedFiles.size / totalFiles) * 100;

      if (coverage < 80) {
        issues.push({
          type: 'performance',
          severity: coverage < 50 ? 'critical' : 'warning',
          message: `Low file coverage: ${coverage.toFixed(1)}% (${indexedFiles.size}/${totalFiles} files)`,
          details: { 
            coverage: coverage,
            indexedFiles: indexedFiles.size,
            totalFiles: totalFiles 
          },
          suggestedAction: 'incremental_update'
        });
      }

      // Check for missing embeddings
      const chunksWithoutEmbeddings = chunks.filter(c => !c.embedding || c.embedding.length === 0);
      if (chunksWithoutEmbeddings.length > 0) {
        const percentage = (chunksWithoutEmbeddings.length / chunks.length) * 100;
        issues.push({
          type: 'performance',
          severity: percentage > 5 ? 'critical' : 'warning',
          message: `${chunksWithoutEmbeddings.length} chunks missing embeddings (${percentage.toFixed(1)}%)`,
          details: { 
            missingCount: chunksWithoutEmbeddings.length,
            totalCount: chunks.length 
          },
          suggestedAction: 'incremental_update'
        });
      }

    } catch (error) {
      issues.push({
        type: 'performance',
        severity: 'warning',
        message: `Performance check failed: ${error instanceof Error ? error.message : 'Unknown error'}`,
        suggestedAction: 'investigate'
      });
    }

    return issues;
  }

  private async getCurrentRepositorySnapshot(): Promise<RepositorySnapshot> {
    // Get current git state
    const currentBranch = await this.gitScanner.getCurrentBranch();
    const latestCommit = await this.gitScanner.getLatestCommit();
    
    // Get dependency file hashes
    const dependencyFiles = ['package.json', 'package-lock.json', 'yarn.lock', 'requirements.txt', 'Cargo.toml', 'go.mod'];
    const buildConfigFiles = ['tsconfig.json', 'webpack.config.js', 'vite.config.js', '.babelrc', 'rollup.config.js'];
    
    const dependencyHashes = await this.getFileHashes(dependencyFiles);
    const buildConfigHashes = await this.getFileHashes(buildConfigFiles);

    return {
      currentBranch,
      latestCommit,
      dependencyHashes,
      buildConfigHashes
    };
  }

  private async getIndexedRepositorySnapshot(): Promise<RepositorySnapshot | null> {
    try {
      const metadata = await this.vectorStore.getMetadata();
      return metadata?.repositorySnapshot || null;
    } catch {
      return null;
    }
  }

  private async getFileHashes(fileNames: string[]): Promise<Map<string, string>> {
    const hashes = new Map<string, string>();
    
    for (const fileName of fileNames) {
      try {
        const filePath = path.join(this.repositoryPath, fileName);
        const content = await fs.readFile(filePath, 'utf-8');
        const hash = crypto.createHash('sha256').update(content).digest('hex');
        hashes.set(fileName, hash);
      } catch {
        // File doesn't exist, skip
      }
    }
    
    return hashes;
  }

  private findChangedDependencies(current: Map<string, string>, indexed: Map<string, string>): string[] {
    const changed: string[] = [];
    
    for (const [file, hash] of current) {
      if (indexed.get(file) !== hash) {
        changed.push(file);
      }
    }
    
    return changed;
  }

  private async getCommitDistance(oldCommit: string, newCommit: string): Promise<number> {
    try {
      return await this.gitScanner.getCommitDistance(oldCommit, newCommit);
    } catch {
      return 0;
    }
  }

  private async getIndexStats() {
    const chunks = await this.vectorStore.getAllChunks();
    const metadata = await this.vectorStore.getMetadata();
    
    return {
      totalChunks: chunks.length,
      totalFiles: new Set(chunks.map(c => c.file_path)).size,
      lastIndexed: metadata?.lastIndexed ? new Date(metadata.lastIndexed) : null,
      indexAge: metadata?.lastIndexed ? Date.now() - metadata.lastIndexed : 0,
      embeddingModel: metadata?.embeddingModel || 'unknown',
      schemaVersion: metadata?.schemaVersion || '1.0.0'
    };
  }

  private determineOverallHealth(issues: HealthIssue[]): 'healthy' | 'degraded' | 'critical' {
    const criticalIssues = issues.filter(i => i.severity === 'critical');
    const warningIssues = issues.filter(i => i.severity === 'warning');
    
    if (criticalIssues.length > 0) return 'critical';
    if (warningIssues.length > 2) return 'degraded';
    if (issues.length > 0) return 'degraded';
    return 'healthy';
  }

  private generateRecommendations(issues: HealthIssue[]): string[] {
    const recommendations: string[] = [];
    
    const needsFullRebuild = issues.some(i => i.suggestedAction === 'full_rebuild');
    const needsIncremental = issues.some(i => i.suggestedAction === 'incremental_update');
    const needsInvestigation = issues.some(i => i.suggestedAction === 'investigate');
    
    if (needsFullRebuild) {
      recommendations.push('🔄 Run full rebuild: npm run rebuild');
    } else if (needsIncremental) {
      recommendations.push('📈 Run incremental update: npm run server');
    }
    
    if (needsInvestigation) {
      recommendations.push('🔍 Manual investigation recommended');
    }
    
    if (issues.length === 0) {
      recommendations.push('✅ Index is healthy, no action needed');
    }
    
    return recommendations;
  }

  // Public method to check if rebuild is recommended
  async shouldRebuild(): Promise<{ shouldRebuild: boolean; reason: string; mode: 'full' | 'incremental' }> {
    const report = await this.performHealthCheck();
    
    const criticalIssues = report.issues.filter(i => i.severity === 'critical');
    const needsFullRebuild = report.issues.some(i => i.suggestedAction === 'full_rebuild');
    
    if (criticalIssues.length > 0) {
      return {
        shouldRebuild: true,
        reason: `Critical issues detected: ${criticalIssues.map(i => i.message).join('; ')}`,
        mode: 'full'
      };
    }
    
    if (needsFullRebuild) {
      return {
        shouldRebuild: true,
        reason: `Build configuration changes detected`,
        mode: 'full'
      };
    }
    
    const needsIncremental = report.issues.some(i => i.suggestedAction === 'incremental_update');
    if (needsIncremental) {
      return {
        shouldRebuild: true,
        reason: `Code changes or staleness detected`,
        mode: 'incremental'
      };
    }
    
    return {
      shouldRebuild: false,
      reason: 'Index is healthy',
      mode: 'incremental'
    };
  }
}