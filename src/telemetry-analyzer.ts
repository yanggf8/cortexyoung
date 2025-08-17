/**
 * Telemetry Analyzer for Claude Code Usage Pattern Analysis
 * 
 * Processes collected telemetry data to generate insights about
 * context window effectiveness and MMR optimization performance.
 */

import { 
  TelemetryEventUnion, 
  ContextRetrievalEvent, 
  UserInteractionEvent, 
  MMRPerformanceEvent,
  ContextEffectivenessMetrics,
  MMROptimizationMetrics 
} from './telemetry-schema';
import { log, warn } from './logging-utils';
import * as fs from 'fs/promises';
import * as path from 'path';

export interface MMRPresetAnalysis {
  preset: string;
  usage_count: number;
  avg_context_quality_score: number;
  avg_follow_up_rate: number;
  avg_token_efficiency: number;
  avg_critical_set_coverage: number;
  effectiveness_ranking: number;
  recommended_use_cases: string[];
}

export interface ContextQualityTrend {
  time_period: string;
  context_quality_score: number;
  follow_up_rate: number;
  token_waste_rate: number;
  critical_set_coverage: number;
  user_satisfaction: number;
}

export interface UsageInsights {
  total_queries: number;
  most_effective_preset: string;
  avg_query_response_time: number;
  common_query_patterns: string[];
  peak_usage_hours: number[];
  quality_trends: ContextQualityTrend[];
  mmr_preset_analysis: MMRPresetAnalysis[];
  optimization_recommendations: string[];
}

export class TelemetryAnalyzer {
  private telemetryDir: string;

  constructor(repositoryPath: string) {
    this.telemetryDir = path.join(repositoryPath, '.cortex', 'telemetry');
  }

  async analyzeUsagePatterns(daysBack: number = 7): Promise<UsageInsights> {
    log(`[TelemetryAnalyzer] Analyzing usage patterns for past ${daysBack} days`);
    
    const events = await this.loadRecentEvents(daysBack);
    
    const contextEvents = events.filter(e => e.event_type === 'context_retrieval') as ContextRetrievalEvent[];
    const interactionEvents = events.filter(e => e.event_type === 'user_interaction') as UserInteractionEvent[];
    const mmrEvents = events.filter(e => e.event_type === 'mmr_performance') as MMRPerformanceEvent[];

    log(`[TelemetryAnalyzer] Processing ${contextEvents.length} context retrievals, ${interactionEvents.length} interactions, ${mmrEvents.length} MMR events`);

    return {
      total_queries: contextEvents.length,
      most_effective_preset: await this.findMostEffectivePreset(contextEvents, interactionEvents),
      avg_query_response_time: this.calculateAverageResponseTime(contextEvents),
      common_query_patterns: this.extractQueryPatterns(contextEvents),
      peak_usage_hours: this.findPeakUsageHours(contextEvents),
      quality_trends: await this.analyzeQualityTrends(contextEvents, interactionEvents, daysBack),
      mmr_preset_analysis: await this.analyzeMMRPresets(contextEvents, interactionEvents, mmrEvents),
      optimization_recommendations: await this.generateOptimizationRecommendations(contextEvents, interactionEvents, mmrEvents)
    };
  }

  async generateContextEffectivenessReport(): Promise<ContextEffectivenessMetrics> {
    const events = await this.loadRecentEvents(7);
    
    const contextEvents = events.filter(e => e.event_type === 'context_retrieval') as ContextRetrievalEvent[];
    const interactionEvents = events.filter(e => e.event_type === 'user_interaction') as UserInteractionEvent[];
    const mmrEvents = events.filter(e => e.event_type === 'mmr_performance') as MMRPerformanceEvent[];

    // Calculate follow-up rate
    const queryIds = new Set(contextEvents.map(e => e.query_id));
    const queriesWithFollowUp = new Set(
      interactionEvents
        .filter(e => e.follow_up_within_5min)
        .map(e => e.query_id)
    );
    const avgFollowUpRate = queryIds.size > 0 ? queriesWithFollowUp.size / queryIds.size : 0;

    // Calculate reference match rate from MMR events
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

    // Calculate user satisfaction
    const satisfactionScores = interactionEvents
      .map(e => e.context_usefulness_score)
      .filter(score => score !== undefined) as number[];
    const avgUserSatisfaction = satisfactionScores.length > 0
      ? satisfactionScores.reduce((sum, score) => sum + score, 0) / satisfactionScores.length / 5
      : 0;

    // Calculate composite quality score
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

  private async loadRecentEvents(daysBack: number): Promise<TelemetryEventUnion[]> {
    const events: TelemetryEventUnion[] = [];
    const cutoffTime = Date.now() - (daysBack * 24 * 60 * 60 * 1000);
    
    try {
      const files = await fs.readdir(this.telemetryDir);
      const jsonlFiles = files.filter(f => f.endsWith('.jsonl'));
      
      for (const file of jsonlFiles) {
        const filepath = path.join(this.telemetryDir, file);
        const content = await fs.readFile(filepath, 'utf-8');
        const lines = content.split('\n').filter(line => line.trim());
        
        for (const line of lines) {
          try {
            const event = JSON.parse(line) as TelemetryEventUnion;
            if (event.timestamp >= cutoffTime) {
              events.push(event);
            }
          } catch (parseError) {
            warn(`[TelemetryAnalyzer] Failed to parse event line: ${parseError}`);
          }
        }
      }
    } catch (error) {
      warn(`[TelemetryAnalyzer] Failed to load events: ${error instanceof Error ? error.message : error}`);
    }
    
    return events.sort((a, b) => a.timestamp - b.timestamp);
  }

  private async findMostEffectivePreset(
    contextEvents: ContextRetrievalEvent[], 
    interactionEvents: UserInteractionEvent[]
  ): Promise<string> {
    const presetScores = new Map<string, { total: number, count: number }>();
    
    for (const contextEvent of contextEvents) {
      const preset = contextEvent.mmr_preset;
      const queryFollowUps = interactionEvents.filter(
        e => e.query_id === contextEvent.query_id && e.follow_up_within_5min
      );
      
      // Score based on inverse follow-up rate and critical set coverage
      const score = (1 - queryFollowUps.length * 0.5) * contextEvent.critical_set_coverage;
      
      if (!presetScores.has(preset)) {
        presetScores.set(preset, { total: 0, count: 0 });
      }
      
      const current = presetScores.get(preset)!;
      current.total += score;
      current.count += 1;
    }
    
    let bestPreset = 'balanced';
    let bestScore = 0;
    
    for (const [preset, data] of presetScores) {
      const avgScore = data.total / data.count;
      if (avgScore > bestScore) {
        bestScore = avgScore;
        bestPreset = preset;
      }
    }
    
    return bestPreset;
  }

  private calculateAverageResponseTime(contextEvents: ContextRetrievalEvent[]): number {
    if (contextEvents.length === 0) return 0;
    
    const totalTime = contextEvents.reduce((sum, e) => sum + e.selection_time_ms, 0);
    return totalTime / contextEvents.length;
  }

  private extractQueryPatterns(contextEvents: ContextRetrievalEvent[]): string[] {
    const queryTypes = new Map<string, number>();
    const queryComplexities = new Map<string, number>();
    
    for (const event of contextEvents) {
      // Count query types
      queryTypes.set(event.query_type, (queryTypes.get(event.query_type) || 0) + 1);
      
      // Categorize by complexity
      const complexity = event.query_complexity_score;
      let complexityCategory: string;
      if (complexity > 0.7) complexityCategory = 'complex';
      else if (complexity > 0.4) complexityCategory = 'medium';
      else complexityCategory = 'simple';
      
      queryComplexities.set(complexityCategory, (queryComplexities.get(complexityCategory) || 0) + 1);
    }
    
    const patterns: string[] = [];
    
    // Most common query types
    const sortedTypes = Array.from(queryTypes.entries()).sort((a, b) => b[1] - a[1]);
    if (sortedTypes.length > 0) {
      patterns.push(`Most common: ${sortedTypes[0][0]} (${sortedTypes[0][1]} queries)`);
    }
    
    // Complexity distribution
    const sortedComplexity = Array.from(queryComplexities.entries()).sort((a, b) => b[1] - a[1]);
    if (sortedComplexity.length > 0) {
      patterns.push(`Complexity: ${sortedComplexity[0][0]} queries dominate`);
    }
    
    return patterns;
  }

  private findPeakUsageHours(contextEvents: ContextRetrievalEvent[]): number[] {
    const hourCounts = new Map<number, number>();
    
    for (const event of contextEvents) {
      const hour = new Date(event.timestamp).getHours();
      hourCounts.set(hour, (hourCounts.get(hour) || 0) + 1);
    }
    
    const sortedHours = Array.from(hourCounts.entries()).sort((a, b) => b[1] - a[1]);
    return sortedHours.slice(0, 3).map(([hour]) => hour);
  }

  private async analyzeQualityTrends(
    contextEvents: ContextRetrievalEvent[],
    interactionEvents: UserInteractionEvent[],
    daysBack: number
  ): Promise<ContextQualityTrend[]> {
    const trends: ContextQualityTrend[] = [];
    const msPerDay = 24 * 60 * 60 * 1000;
    const now = Date.now();
    
    for (let i = 0; i < daysBack; i++) {
      const dayStart = now - (i + 1) * msPerDay;
      const dayEnd = now - i * msPerDay;
      
      const dayContextEvents = contextEvents.filter(e => e.timestamp >= dayStart && e.timestamp < dayEnd);
      const dayInteractionEvents = interactionEvents.filter(e => e.timestamp >= dayStart && e.timestamp < dayEnd);
      
      if (dayContextEvents.length === 0) continue;
      
      const queryIds = new Set(dayContextEvents.map(e => e.query_id));
      const followUps = dayInteractionEvents.filter(e => e.follow_up_within_5min).length;
      const followUpRate = queryIds.size > 0 ? followUps / queryIds.size : 0;
      
      const avgCriticalCoverage = dayContextEvents.reduce((sum, e) => sum + e.critical_set_coverage, 0) / dayContextEvents.length;
      const avgTokenUtilization = dayContextEvents.reduce((sum, e) => sum + e.token_budget_utilization, 0) / dayContextEvents.length;
      
      const contextQualityScore = (
        (1 - followUpRate) * 0.4 +
        avgCriticalCoverage * 0.35 +
        avgTokenUtilization * 0.25
      );
      
      trends.push({
        time_period: new Date(dayStart).toISOString().split('T')[0],
        context_quality_score: contextQualityScore,
        follow_up_rate: followUpRate,
        token_waste_rate: 1 - avgTokenUtilization,
        critical_set_coverage: avgCriticalCoverage,
        user_satisfaction: 0 // Would need user feedback data
      });
    }
    
    return trends.reverse(); // Oldest first
  }

  private async analyzeMMRPresets(
    contextEvents: ContextRetrievalEvent[],
    interactionEvents: UserInteractionEvent[],
    mmrEvents: MMRPerformanceEvent[]
  ): Promise<MMRPresetAnalysis[]> {
    const presetData = new Map<string, {
      count: number;
      totalQualityScore: number;
      totalFollowUpRate: number;
      totalTokenEfficiency: number;
      totalCriticalCoverage: number;
    }>();
    
    for (const contextEvent of contextEvents) {
      const preset = contextEvent.mmr_preset;
      
      if (!presetData.has(preset)) {
        presetData.set(preset, {
          count: 0,
          totalQualityScore: 0,
          totalFollowUpRate: 0,
          totalTokenEfficiency: 0,
          totalCriticalCoverage: 0
        });
      }
      
      const data = presetData.get(preset)!;
      data.count += 1;
      
      // Calculate follow-up rate for this query
      const queryFollowUps = interactionEvents.filter(
        e => e.query_id === contextEvent.query_id && e.follow_up_within_5min
      ).length;
      const followUpRate = queryFollowUps > 0 ? 1 : 0;
      
      data.totalFollowUpRate += followUpRate;
      data.totalTokenEfficiency += contextEvent.token_budget_utilization;
      data.totalCriticalCoverage += contextEvent.critical_set_coverage;
      
      // Calculate quality score
      const qualityScore = (
        (1 - followUpRate) * 0.4 +
        contextEvent.critical_set_coverage * 0.35 +
        contextEvent.token_budget_utilization * 0.25
      );
      data.totalQualityScore += qualityScore;
    }
    
    const analyses: MMRPresetAnalysis[] = [];
    let rank = 1;
    
    // Sort by quality score for ranking
    const sortedPresets = Array.from(presetData.entries()).sort(
      (a, b) => (b[1].totalQualityScore / b[1].count) - (a[1].totalQualityScore / a[1].count)
    );
    
    for (const [preset, data] of sortedPresets) {
      const useCase = this.getPresetUseCases(preset);
      
      analyses.push({
        preset,
        usage_count: data.count,
        avg_context_quality_score: data.totalQualityScore / data.count,
        avg_follow_up_rate: data.totalFollowUpRate / data.count,
        avg_token_efficiency: data.totalTokenEfficiency / data.count,
        avg_critical_set_coverage: data.totalCriticalCoverage / data.count,
        effectiveness_ranking: rank++,
        recommended_use_cases: useCase
      });
    }
    
    return analyses;
  }

  private getPresetUseCases(preset: string): string[] {
    switch (preset) {
      case 'balanced':
        return ['General code analysis', 'Bug investigation', 'Feature development'];
      case 'high-relevance':
        return ['Specific function analysis', 'Focused debugging', 'Code review'];
      case 'high-diversity':
        return ['Architecture exploration', 'Codebase overview', 'Refactoring planning'];
      default:
        return ['Custom use cases'];
    }
  }

  private async generateOptimizationRecommendations(
    contextEvents: ContextRetrievalEvent[],
    interactionEvents: UserInteractionEvent[],
    mmrEvents: MMRPerformanceEvent[]
  ): Promise<string[]> {
    const recommendations: string[] = [];
    
    if (contextEvents.length === 0) {
      recommendations.push('Insufficient data for recommendations - need more usage patterns');
      return recommendations;
    }
    
    // Analyze follow-up rates
    const totalQueries = new Set(contextEvents.map(e => e.query_id)).size;
    const followUpQueries = new Set(
      interactionEvents.filter(e => e.follow_up_within_5min).map(e => e.query_id)
    ).size;
    const followUpRate = totalQueries > 0 ? followUpQueries / totalQueries : 0;
    
    if (followUpRate > 0.3) {
      recommendations.push('High follow-up rate detected - consider tuning MMR lambda for better initial relevance');
    }
    
    // Analyze critical set coverage
    const avgCriticalCoverage = contextEvents.reduce((sum, e) => sum + e.critical_set_coverage, 0) / contextEvents.length;
    if (avgCriticalCoverage < 0.9) {
      recommendations.push('Low critical set coverage - consider increasing token budget or adjusting dependency traversal depth');
    }
    
    // Analyze token efficiency
    const avgTokenUtilization = contextEvents.reduce((sum, e) => sum + e.token_budget_utilization, 0) / contextEvents.length;
    if (avgTokenUtilization < 0.6) {
      recommendations.push('Low token utilization - consider reducing token budget or improving chunking strategy');
    } else if (avgTokenUtilization > 0.95) {
      recommendations.push('Very high token utilization - consider increasing token budget to avoid truncation');
    }
    
    // Analyze response times
    const avgResponseTime = this.calculateAverageResponseTime(contextEvents);
    if (avgResponseTime > 2000) {
      recommendations.push('Slow response times detected - consider optimizing MMR selection or caching strategies');
    }
    
    return recommendations;
  }
}