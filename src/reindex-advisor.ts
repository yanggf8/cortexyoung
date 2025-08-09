import { SchemaValidator } from './schema-validator';
import { EmbeddingGenerator } from './embedder';
import { IndexHealthChecker } from './index-health-checker';
import { PersistentVectorStore } from './persistent-vector-store';
import { ModelInfo } from './types';

export interface ReindexRecommendation {
  recommend: boolean;
  reason: string;
  severity: 'low' | 'medium' | 'high' | 'critical';
  category: 'schema' | 'model' | 'corruption' | 'performance' | 'staleness';
  autoReindex?: boolean; // Should we automatically trigger reindex?
  details?: any; // Additional context for the recommendation
}

export class ReindexAdvisor {
  private embedder: EmbeddingGenerator;
  private healthChecker: IndexHealthChecker;

  constructor(embedder: EmbeddingGenerator) {
    this.embedder = embedder;
    // Health checker will be initialized when needed
    this.healthChecker = null as any;
  }

  /**
   * Analyzes the current index state and provides reindex recommendations
   */
  async analyzeReindexNeeds(
    vectorStore: PersistentVectorStore,
    repositoryPath: string
  ): Promise<ReindexRecommendation[]> {
    const recommendations: ReindexRecommendation[] = [];

    // Check schema compatibility
    const schemaRec = await this.checkSchemaCompatibility(vectorStore);
    if (schemaRec) recommendations.push(schemaRec);

    // Check model compatibility
    const modelRec = await this.checkModelCompatibility(vectorStore);
    if (modelRec) recommendations.push(modelRec);

    // Check for corruption
    const corruptionRec = await this.checkCorruption(vectorStore, repositoryPath);
    if (corruptionRec) recommendations.push(corruptionRec);

    // Check performance/staleness
    const performanceRec = await this.checkPerformance(vectorStore);
    if (performanceRec) recommendations.push(performanceRec);

    return recommendations;
  }

  /**
   * Provides analysis recommendations without automatically deciding modes
   * Mode switching should only happen for the three legitimate cases:
   * 1. First time (no existing embeddings)
   * 2. User explicit request (mode: 'full' or 'reindex')
   * 3. Complete corruption (embedding files missing/unreadable)
   */
  async getReindexRecommendation(
    vectorStore: PersistentVectorStore,
    repositoryPath: string
  ): Promise<{
    shouldReindex: boolean;
    mode: 'incremental' | 'reindex';
    primaryReason: string;
    allRecommendations: ReindexRecommendation[];
    forcedRebuildRequired: boolean; // Only true for legitimate corruption cases
  }> {
    const recommendations = await this.analyzeReindexNeeds(vectorStore, repositoryPath);
    
    // Sort by severity
    recommendations.sort((a, b) => {
      const severityOrder = { critical: 4, high: 3, medium: 2, low: 1 };
      return severityOrder[b.severity] - severityOrder[a.severity];
    });

    // Only flag forced rebuild for TRUE corruption cases
    const forcedRebuildRequired = recommendations.some(r => 
      r.category === 'corruption' && 
      r.severity === 'critical' &&
      (r.reason.includes('duplicate chunk IDs') || r.reason.includes('Health check failed'))
    );

    const criticalOrHigh = recommendations.filter(r => 
      r.recommend && (r.severity === 'critical' || r.severity === 'high')
    );

    if (criticalOrHigh.length > 0) {
      return {
        shouldReindex: true,
        mode: 'incremental', // Let user decide mode, don't override automatically
        primaryReason: criticalOrHigh[0].reason,
        allRecommendations: recommendations,
        forcedRebuildRequired
      };
    }

    const medium = recommendations.filter(r => r.recommend && r.severity === 'medium');
    if (medium.length > 0) {
      return {
        shouldReindex: true,
        mode: 'incremental',
        primaryReason: medium[0].reason,
        allRecommendations: recommendations,
        forcedRebuildRequired: false
      };
    }

    return {
      shouldReindex: false,
      mode: 'incremental',
      primaryReason: 'Index is in good condition',
      allRecommendations: recommendations,
      forcedRebuildRequired: false
    };
  }

  private async checkSchemaCompatibility(vectorStore: PersistentVectorStore): Promise<ReindexRecommendation | null> {
    try {
      const metadata = await vectorStore.getMetadata();
      if (!metadata) return null;

      const schemaVersion = metadata.schemaVersion || '1.0.0';
      const schemaInfo = SchemaValidator.validateSchema(schemaVersion);
      
      if (!schemaInfo.compatible) {
        return {
          recommend: true,
          reason: `Schema version ${schemaVersion} is incompatible with program version 2.1.0. Compatible schemas: ${['1.0.0', '1.1.0'].join(', ')}`,
          severity: 'high',
          category: 'schema',
          autoReindex: false // Never auto-reindex, let user decide
        };
      }

      // Schema is compatible, no reindex needed
      return null;
    } catch (error) {
      console.warn('Failed to check schema compatibility:', error);
    }

    return null;
  }

  private async checkModelCompatibility(vectorStore: PersistentVectorStore): Promise<ReindexRecommendation | null> {
    try {
      const metadata = await vectorStore.getMetadata();
      const cachedModelInfo = metadata?.modelInfo;

      const compatibility = await this.embedder.validateModelCompatibility(cachedModelInfo);

      if (!compatibility.compatible && compatibility.recommendation === 'reindex') {
        return {
          recommend: true,
          reason: compatibility.reason,
          severity: 'high',
          category: 'model',
          autoReindex: false // Never auto-reindex, let user decide
        };
      }
    } catch (error) {
      console.warn('Failed to check model compatibility:', error);
    }

    return null;
  }

  private async checkCorruption(
    vectorStore: PersistentVectorStore, 
    repositoryPath: string
  ): Promise<ReindexRecommendation | null> {
    try {
      // Initialize health checker with proper dependencies
      this.healthChecker = new IndexHealthChecker(repositoryPath, vectorStore);
      const healthReport = await this.healthChecker.performHealthCheck();

      const criticalIssues = healthReport.issues.filter(i => i.severity === 'critical');
      if (criticalIssues.length > 0) {
        return {
          recommend: true,
          reason: `Critical index issues detected: ${criticalIssues.map(i => i.message).join(', ')}`,
          severity: 'critical',
          category: 'corruption',
          autoReindex: false // Even corruption should not auto-reindex, let user decide
        };
      }

      const warningIssues = healthReport.issues.filter(i => i.severity === 'warning');
      if (warningIssues.length > 0) {
        return {
          recommend: true,
          reason: `Index issues detected: ${warningIssues.map(i => i.message).join(', ')}`,
          severity: 'medium',
          category: 'corruption',
          autoReindex: false
        };
      }
    } catch (error) {
      console.warn('Failed to check index corruption:', error);
    }

    return null;
  }

  private async checkPerformance(vectorStore: PersistentVectorStore): Promise<ReindexRecommendation | null> {
    try {
      const metadata = await vectorStore.getMetadata();
      if (!metadata || !metadata.lastIndexed) return null;

      const daysSinceIndexed = (Date.now() - metadata.lastIndexed) / (1000 * 60 * 60 * 24);

      // Recommend reindex if it's been more than 30 days
      if (daysSinceIndexed > 30) {
        return {
          recommend: true,
          reason: `Index is ${Math.round(daysSinceIndexed)} days old. Consider refreshing for optimal performance.`,
          severity: 'low',
          category: 'staleness',
          autoReindex: false
        };
      }

      // Check chunk count vs file count ratio for performance issues
      const stats = await vectorStore.getStats();
      if (stats.total_chunks > 10000) {
        return {
          recommend: true,
          reason: `Large index (${stats.total_chunks} chunks) may benefit from optimization`,
          severity: 'low',
          category: 'performance',
          autoReindex: false
        };
      }
    } catch (error) {
      console.warn('Failed to check performance metrics:', error);
    }

    return null;
  }

  /**
   * Formats recommendations for user display
   */
  static formatRecommendations(recommendations: ReindexRecommendation[]): string {
    if (recommendations.length === 0) {
      return '✅ Index is in good condition - no reindex needed';
    }

    const lines: string[] = [];
    const critical = recommendations.filter(r => r.severity === 'critical');
    const high = recommendations.filter(r => r.severity === 'high');
    const medium = recommendations.filter(r => r.severity === 'medium');
    const low = recommendations.filter(r => r.severity === 'low');

    if (critical.length > 0) {
      lines.push('🚨 CRITICAL ISSUES:');
      critical.forEach(r => lines.push(`   • ${r.reason}`));
      lines.push('');
    }

    if (high.length > 0) {
      lines.push('⚠️  HIGH PRIORITY:');
      high.forEach(r => lines.push(`   • ${r.reason}`));
      lines.push('');
    }

    if (medium.length > 0) {
      lines.push('📋 MEDIUM PRIORITY:');
      medium.forEach(r => lines.push(`   • ${r.reason}`));
      lines.push('');
    }

    if (low.length > 0) {
      lines.push('💡 SUGGESTIONS:');
      low.forEach(r => lines.push(`   • ${r.reason}`));
    }

    return lines.join('\n');
  }
}