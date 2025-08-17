/**
 * Telemetry Schema for Claude Code Usage Pattern Monitoring
 * 
 * Purpose: Track context window effectiveness and validate semantic system impact
 * on code understanding quality. Measures whether MMR + relationship scoring
 * delivers concise, consistent, complete program information.
 */

// Base event interface
export interface TelemetryEvent {
  event_id: string;
  session_id: string;
  timestamp: number;
  event_type: string;
  repository_hash: string; // Anonymized repo identifier
  user_hash?: string; // Optional anonymized user identifier
}

// Session lifecycle events
export interface SessionStartEvent extends TelemetryEvent {
  event_type: 'session_start';
  startup_kind: 'cold' | 'warm';
  startup_duration_ms: number;
  total_chunks_loaded: number;
  mmr_preset: string;
  cortex_version: string;
}

export interface SessionEndEvent extends TelemetryEvent {
  event_type: 'session_end';
  session_duration_ms: number;
  total_queries: number;
  total_context_provided: number;
}

// Core context retrieval events
export interface ContextRetrievalEvent extends TelemetryEvent {
  event_type: 'context_retrieval';
  query_id: string;
  query_type: 'semantic_search' | 'code_intelligence' | 'relationship_analysis' | 'trace_execution_path' | 'find_code_patterns';
  
  // MMR Configuration
  mmr_preset: 'balanced' | 'high-relevance' | 'high-diversity' | 'custom';
  lambda_relevance: number;
  diversity_metric: 'cosine' | 'jaccard' | 'semantic';
  max_token_budget: number;
  
  // Search parameters
  max_chunks_requested: number;
  query_complexity_score: number; // 0-1 based on query length and specificity
  
  // Candidate pool metrics
  total_candidates: number;
  candidates_after_filtering: number;
  dependency_chain_length: number;
  
  // Critical set analysis
  critical_set_files: number;
  critical_set_functions: number;
  critical_set_confidence: number;
  critical_chunks_selected: number;
  
  // MMR selection results
  chunks_selected: number;
  total_tokens_provided: number;
  token_budget_utilization: number; // 0-1
  critical_set_coverage: number; // 0-1
  diversity_score: number; // 0-1
  selection_time_ms: number;
  
  // Context composition
  files_included: number;
  unique_file_types: string[]; // ['.ts', '.js', '.json']
  relationship_types_included: string[]; // ['imports', 'calls', 'extends']
}

// Context quality metrics
export interface ContextQualityEvent extends TelemetryEvent {
  event_type: 'context_quality';
  query_id: string;
  
  // Content analysis
  code_to_comment_ratio: number;
  function_to_variable_ratio: number;
  avg_function_complexity: number;
  dependency_depth_distribution: number[]; // Histogram of dependency depths
  
  // Redundancy detection
  duplicate_content_percentage: number;
  semantic_similarity_variance: number;
  file_overlap_percentage: number;
  
  // Coverage metrics
  essential_patterns_covered: string[]; // ['error_handling', 'data_flow', 'api_integration']
  missing_dependency_indicators: number;
  context_completeness_score: number; // 0-1
}

// User interaction tracking (post-context provision)
export interface UserInteractionEvent extends TelemetryEvent {
  event_type: 'user_interaction';
  query_id: string;
  interaction_type: 'file_opened' | 'code_copied' | 'follow_up_query' | 'scope_expansion' | 'task_completion';
  
  // File interaction details
  file_path_hash?: string; // Anonymized file path
  file_was_in_context: boolean;
  time_since_context_ms: number;
  
  // Follow-up behavior
  follow_up_count?: number;
  follow_up_within_5min?: boolean;
  query_refinement_type?: 'clarification' | 'expansion' | 'pivot' | 'drill_down';
  
  // Task completion indicators
  task_success_indicators?: string[]; // ['test_passed', 'code_executed', 'problem_resolved']
  context_usefulness_score?: number; // 1-5 if available from user feedback
}

// MMR Performance Analysis
export interface MMRPerformanceEvent extends TelemetryEvent {
  event_type: 'mmr_performance';
  query_id: string;
  
  // Comparative analysis (if A/B testing)
  preset_comparison?: {
    preset_a: string;
    preset_b: string;
    winner: string;
    confidence: number;
  };
  
  // Performance metrics
  relevance_vs_diversity_tradeoff: number; // -1 to 1 (diversity bias to relevance bias)
  selection_efficiency: number; // chunks selected / total candidates
  token_efficiency: number; // useful tokens / total tokens
  
  // Quality indicators
  reference_match_rate: number; // Files referenced by Claude / files provided
  follow_up_reduction_rate: number; // Compared to baseline
  context_reuse_rate: number; // How often same context helps multiple queries
}

// System health and performance
export interface SystemPerformanceEvent extends TelemetryEvent {
  event_type: 'system_performance';
  
  // Performance metrics
  avg_query_response_time_ms: number;
  embedding_cache_hit_rate: number;
  index_freshness_score: number; // 0-1 based on how up-to-date the index is
  
  // Resource utilization
  memory_usage_mb: number;
  cpu_utilization_percent: number;
  concurrent_queries: number;
  
  // Error rates
  mmr_selection_failures: number;
  relationship_resolution_failures: number;
  context_truncation_incidents: number;
}

// Aggregated insights (daily/weekly summaries)
export interface UsageInsightEvent extends TelemetryEvent {
  event_type: 'usage_insight';
  time_period: 'daily' | 'weekly' | 'monthly';
  
  // Usage patterns
  most_effective_mmr_preset: string;
  avg_context_utilization: number;
  common_query_patterns: string[];
  peak_usage_hours: number[];
  
  // Quality trends
  context_quality_trend: 'improving' | 'stable' | 'declining';
  user_satisfaction_trend: 'improving' | 'stable' | 'declining';
  follow_up_rate_trend: number; // Percentage change
  
  // System insights
  optimal_token_budget_range: [number, number];
  most_valuable_relationship_types: string[];
  underutilized_features: string[];
}

// Configuration for telemetry collection
export interface TelemetryConfig {
  enabled: boolean;
  collection_endpoint?: string;
  sampling_rate: number; // 0-1, for performance
  anonymization_level: 'minimal' | 'standard' | 'strict';
  retention_days: number;
  
  // Event-specific settings
  track_session_events: boolean;
  track_context_retrieval: boolean;
  track_user_interactions: boolean;
  track_mmr_performance: boolean;
  track_system_performance: boolean;
  
  // Privacy settings
  exclude_file_contents: boolean;
  exclude_query_text: boolean;
  hash_file_paths: boolean;
  anonymize_timestamps: boolean;
}

// Utility types for analysis
export type TelemetryEventUnion = 
  | SessionStartEvent 
  | SessionEndEvent
  | ContextRetrievalEvent 
  | ContextQualityEvent
  | UserInteractionEvent 
  | MMRPerformanceEvent
  | SystemPerformanceEvent
  | UsageInsightEvent;

// Metrics aggregation helpers
export interface ContextEffectivenessMetrics {
  avg_follow_up_rate: number;
  avg_reference_match_rate: number;
  avg_token_waste_rate: number;
  avg_critical_set_coverage: number;
  avg_user_satisfaction: number;
  context_quality_score: number; // Composite 0-1 score
}

export interface MMROptimizationMetrics {
  preset_performance_ranking: Array<{
    preset: string;
    effectiveness_score: number;
    use_case_fit: string[];
  }>;
  optimal_lambda_range: [number, number];
  best_diversity_metric_by_task: Record<string, string>;
  token_budget_recommendations: Record<string, number>;
}