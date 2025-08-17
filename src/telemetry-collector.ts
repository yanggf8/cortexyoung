/**
 * Telemetry Collector for Claude Code Usage Pattern Monitoring
 * 
 * Lightweight, privacy-focused telemetry collection to validate
 * context window effectiveness and MMR optimization impact.
 */

import { 
  TelemetryEvent, 
  TelemetryEventUnion, 
  TelemetryConfig,
  ContextRetrievalEvent,
  UserInteractionEvent,
  MMRPerformanceEvent,
  SystemPerformanceEvent,
  ContextEffectivenessMetrics
} from './telemetry-schema';
import { log, warn, error } from './logging-utils';
import * as crypto from 'crypto';
import * as fs from 'fs/promises';
import * as path from 'path';

export class TelemetryCollector {
  private config: TelemetryConfig;
  private sessionId: string;
  private repositoryHash: string;
  private eventBuffer: TelemetryEventUnion[] = [];
  private readonly bufferFlushSize = 50;
  private readonly bufferFlushIntervalMs = 60000; // 1 minute
  private flushTimer?: NodeJS.Timeout;

  constructor(repositoryPath: string, config?: Partial<TelemetryConfig>) {
    this.config = {
      enabled: process.env.CORTEX_TELEMETRY_ENABLED !== 'false',
      sampling_rate: parseFloat(process.env.CORTEX_TELEMETRY_SAMPLE_RATE || '1.0'),
      anonymization_level: (process.env.CORTEX_TELEMETRY_ANONYMIZATION as any) || 'standard',
      retention_days: parseInt(process.env.CORTEX_TELEMETRY_RETENTION_DAYS || '30'),
      
      track_session_events: true,
      track_context_retrieval: true,
      track_user_interactions: true,
      track_mmr_performance: true,
      track_system_performance: true,
      
      exclude_file_contents: true,
      exclude_query_text: true,
      hash_file_paths: true,
      anonymize_timestamps: false,
      
      ...config
    };

    this.sessionId = this.generateSessionId();
    this.repositoryHash = this.hashRepositoryPath(repositoryPath);
    
    if (this.config.enabled) {
      this.startPeriodicFlush();
      log(`[Telemetry] Collector initialized session=${this.sessionId.substring(0, 8)} repo=${this.repositoryHash.substring(0, 8)}`);
    }
  }

  // Core event collection methods
  async trackContextRetrieval(params: {
    queryId: string;
    queryType: string;
    mmrConfig: any;
    searchParams: any;
    candidateMetrics: any;
    criticalSetMetrics: any;
    mmrResults: any;
    contextComposition: any;
    timing: any;
  }): Promise<void> {
    if (!this.shouldCollect('track_context_retrieval')) return;

    const event: ContextRetrievalEvent = {
      ...this.createBaseEvent(),
      event_type: 'context_retrieval' as const,
      query_id: params.queryId,
      query_type: params.queryType as any,
      
      mmr_preset: params.mmrConfig.preset || 'balanced',
      lambda_relevance: params.mmrConfig.lambdaRelevance || 0.7,
      diversity_metric: params.mmrConfig.diversityMetric || 'semantic',
      max_token_budget: params.mmrConfig.maxTokenBudget || 100000,
      
      max_chunks_requested: params.searchParams.maxChunks || 20,
      query_complexity_score: this.calculateQueryComplexity(params.searchParams.query),
      
      total_candidates: params.candidateMetrics.total || 0,
      candidates_after_filtering: params.candidateMetrics.filtered || 0,
      dependency_chain_length: params.candidateMetrics.chainLength || 0,
      
      critical_set_files: params.criticalSetMetrics.files || 0,
      critical_set_functions: params.criticalSetMetrics.functions || 0,
      critical_set_confidence: params.criticalSetMetrics.confidence || 0,
      critical_chunks_selected: params.criticalSetMetrics.chunksSelected || 0,
      
      chunks_selected: params.mmrResults.chunksSelected || 0,
      total_tokens_provided: params.mmrResults.totalTokens || 0,
      token_budget_utilization: params.mmrResults.budgetUtilization || 0,
      critical_set_coverage: params.mmrResults.criticalSetCoverage || 0,
      diversity_score: params.mmrResults.diversityScore || 0,
      selection_time_ms: params.timing.selectionTimeMs || 0,
      
      files_included: params.contextComposition.filesIncluded || 0,
      unique_file_types: params.contextComposition.fileTypes || [],
      relationship_types_included: params.contextComposition.relationshipTypes || []
    };

    await this.collectEvent(event);
  }

  async trackUserInteraction(params: {
    queryId: string;
    interactionType: string;
    filePath?: string;
    fileWasInContext: boolean;
    timeSinceContextMs: number;
    followUpData?: any;
    taskCompletionData?: any;
  }): Promise<void> {
    if (!this.shouldCollect('track_user_interactions')) return;

    const event: UserInteractionEvent = {
      ...this.createBaseEvent(),
      event_type: 'user_interaction' as const,
      query_id: params.queryId,
      interaction_type: params.interactionType as any,
      file_path_hash: params.filePath ? this.hashFilePath(params.filePath) : undefined,
      file_was_in_context: params.fileWasInContext,
      time_since_context_ms: params.timeSinceContextMs,
      follow_up_count: params.followUpData?.count,
      follow_up_within_5min: params.followUpData?.within5min,
      query_refinement_type: params.followUpData?.refinementType as any,
      task_success_indicators: params.taskCompletionData?.successIndicators,
      context_usefulness_score: params.taskCompletionData?.usefulnessScore
    };

    await this.collectEvent(event);
  }

  async trackMMRPerformance(params: {
    queryId: string;
    mmrMetrics: any;
    qualityIndicators: any;
    comparisonData?: any;
  }): Promise<void> {
    if (!this.shouldCollect('track_mmr_performance')) return;

    const event: MMRPerformanceEvent = {
      ...this.createBaseEvent(),
      event_type: 'mmr_performance' as const,
      query_id: params.queryId,
      preset_comparison: params.comparisonData,
      relevance_vs_diversity_tradeoff: params.mmrMetrics.tradeoff || 0,
      selection_efficiency: params.mmrMetrics.efficiency || 0,
      token_efficiency: params.mmrMetrics.tokenEfficiency || 0,
      reference_match_rate: params.qualityIndicators.referenceMatchRate || 0,
      follow_up_reduction_rate: params.qualityIndicators.followUpReductionRate || 0,
      context_reuse_rate: params.qualityIndicators.contextReuseRate || 0
    };

    await this.collectEvent(event);
  }

  async trackSystemPerformance(metrics: {
    avgResponseTime: number;
    cacheHitRate: number;
    indexFreshness: number;
    resourceUsage: any;
    errorRates: any;
  }): Promise<void> {
    if (!this.shouldCollect('track_system_performance')) return;

    const event: SystemPerformanceEvent = {
      ...this.createBaseEvent(),
      event_type: 'system_performance' as const,
      avg_query_response_time_ms: metrics.avgResponseTime,
      embedding_cache_hit_rate: metrics.cacheHitRate,
      index_freshness_score: metrics.indexFreshness,
      memory_usage_mb: metrics.resourceUsage.memoryMB || 0,
      cpu_utilization_percent: metrics.resourceUsage.cpuPercent || 0,
      concurrent_queries: metrics.resourceUsage.concurrentQueries || 0,
      mmr_selection_failures: metrics.errorRates.mmrFailures || 0,
      relationship_resolution_failures: metrics.errorRates.relationshipFailures || 0,
      context_truncation_incidents: metrics.errorRates.truncationIncidents || 0
    };

    await this.collectEvent(event);
  }

  // Analytics and insights
  async calculateContextEffectiveness(timeRangeMs: number = 24 * 60 * 60 * 1000): Promise<ContextEffectivenessMetrics> {
    const events = await this.getRecentEvents(timeRangeMs);
    
    const contextEvents = events.filter(e => e.event_type === 'context_retrieval') as ContextRetrievalEvent[];
    const interactionEvents = events.filter(e => e.event_type === 'user_interaction') as UserInteractionEvent[];
    const mmrEvents = events.filter(e => e.event_type === 'mmr_performance') as MMRPerformanceEvent[];

    // Calculate follow-up rate
    const queriesWithFollowUp = interactionEvents.filter(e => e.follow_up_within_5min).length;
    const totalQueries = contextEvents.length;
    const avgFollowUpRate = totalQueries > 0 ? queriesWithFollowUp / totalQueries : 0;

    // Calculate reference match rate
    const avgReferenceMatchRate = mmrEvents.length > 0 
      ? mmrEvents.reduce((sum, e) => sum + e.reference_match_rate, 0) / mmrEvents.length 
      : 0;

    // Calculate token waste rate
    const avgTokenWasteRate = 1 - (mmrEvents.length > 0 
      ? mmrEvents.reduce((sum, e) => sum + e.token_efficiency, 0) / mmrEvents.length 
      : 0);

    // Calculate critical set coverage
    const avgCriticalSetCoverage = contextEvents.length > 0
      ? contextEvents.reduce((sum, e) => sum + e.critical_set_coverage, 0) / contextEvents.length
      : 0;

    // Calculate user satisfaction (if available)
    const satisfactionScores = interactionEvents
      .map(e => e.context_usefulness_score)
      .filter(score => score !== undefined) as number[];
    const avgUserSatisfaction = satisfactionScores.length > 0
      ? satisfactionScores.reduce((sum, score) => sum + score, 0) / satisfactionScores.length / 5 // Normalize to 0-1
      : 0;

    // Composite quality score
    const contextQualityScore = (
      (1 - avgFollowUpRate) * 0.3 +
      avgReferenceMatchRate * 0.25 +
      (1 - avgTokenWasteRate) * 0.2 +
      avgCriticalSetCoverage * 0.15 +
      avgUserSatisfaction * 0.1
    );

    return {
      avg_follow_up_rate: avgFollowUpRate,
      avg_reference_match_rate: avgReferenceMatchRate,
      avg_token_waste_rate: avgTokenWasteRate,
      avg_critical_set_coverage: avgCriticalSetCoverage,
      avg_user_satisfaction: avgUserSatisfaction,
      context_quality_score: contextQualityScore
    };
  }

  // Utility methods
  private createBaseEvent(): Omit<TelemetryEvent, 'event_type'> {
    return {
      event_id: this.generateEventId(),
      session_id: this.sessionId,
      timestamp: this.config.anonymize_timestamps ? 
        Math.floor(Date.now() / (5 * 60 * 1000)) * (5 * 60 * 1000) : // Round to 5-minute intervals
        Date.now(),
      repository_hash: this.repositoryHash
    };
  }

  private shouldCollect(eventTypeSetting: keyof TelemetryConfig): boolean {
    return this.config.enabled && 
           Boolean(this.config[eventTypeSetting]) && 
           Math.random() < this.config.sampling_rate;
  }

  private async collectEvent(event: TelemetryEventUnion): Promise<void> {
    try {
      this.eventBuffer.push(event);
      
      if (this.eventBuffer.length >= this.bufferFlushSize) {
        await this.flushEvents();
      }
    } catch (err) {
      warn(`[Telemetry] Failed to collect event: ${err instanceof Error ? err.message : err}`);
    }
  }

  private async flushEvents(): Promise<void> {
    if (this.eventBuffer.length === 0) return;

    try {
      const events = [...this.eventBuffer];
      this.eventBuffer = [];

      // Write to local file (can be extended to send to remote endpoint)
      await this.writeEventsToFile(events);
      
      log(`[Telemetry] Flushed ${events.length} events`);
    } catch (err) {
      error(`[Telemetry] Failed to flush events: ${err instanceof Error ? err.message : err}`);
    }
  }

  private async writeEventsToFile(events: TelemetryEventUnion[]): Promise<void> {
    const telemetryDir = path.join(process.cwd(), '.cortex', 'telemetry');
    await fs.mkdir(telemetryDir, { recursive: true });
    
    const filename = `telemetry-${new Date().toISOString().split('T')[0]}.jsonl`;
    const filepath = path.join(telemetryDir, filename);
    
    const lines = events.map(event => JSON.stringify(event)).join('\n') + '\n';
    await fs.appendFile(filepath, lines);
  }

  private async getRecentEvents(timeRangeMs: number): Promise<TelemetryEventUnion[]> {
    // Simplified implementation - in production, might query from database
    const cutoffTime = Date.now() - timeRangeMs;
    return this.eventBuffer.filter(event => event.timestamp >= cutoffTime);
  }

  private startPeriodicFlush(): void {
    this.flushTimer = setInterval(() => {
      this.flushEvents().catch(err => 
        warn(`[Telemetry] Periodic flush failed: ${err instanceof Error ? err.message : err}`)
      );
    }, this.bufferFlushIntervalMs);
  }

  private calculateQueryComplexity(query: string): number {
    // Simple heuristic - can be made more sophisticated
    const length = query.length;
    const words = query.split(/\s+/).length;
    const hasSpecialTerms = /\b(class|function|import|export|async|await|error|bug|fix)\b/i.test(query);
    
    let complexity = Math.min(1.0, length / 200); // Length factor
    complexity += Math.min(0.3, words / 20); // Word count factor
    if (hasSpecialTerms) complexity += 0.2; // Technical terms bonus
    
    return Math.min(1.0, complexity);
  }

  private generateSessionId(): string {
    return crypto.randomBytes(16).toString('hex');
  }

  private generateEventId(): string {
    return crypto.randomBytes(8).toString('hex');
  }

  private hashRepositoryPath(repositoryPath: string): string {
    return crypto.createHash('sha256').update(repositoryPath).digest('hex').substring(0, 16);
  }

  private hashFilePath(filePath: string): string {
    return crypto.createHash('sha256').update(filePath).digest('hex').substring(0, 12);
  }

  async shutdown(): Promise<void> {
    if (this.flushTimer) {
      clearInterval(this.flushTimer);
    }
    await this.flushEvents();
    log('[Telemetry] Collector shutdown complete');
  }
}