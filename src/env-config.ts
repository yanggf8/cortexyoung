/**
 * Environment Configuration Utility
 * 
 * Provides centralized environment variable handling with:
 * - CORTEX_ prefixed variables (preferred)
 * - Backward compatibility with unprefixed variables
 * - Type-safe configuration access
 * - Default values
 */

export interface CortexConfig {
  // Core Configuration
  port: number;
  logFile?: string;
  debug: boolean;

  // Advanced Configuration  
  disableRealTime: boolean;
  enableNewLogging: boolean;
  indexMode?: 'full' | 'incremental' | 'reindex';
  forceRebuild: boolean;

  // Embedding & Processing
  embeddingStrategy: 'auto' | 'cached' | 'process-pool';
  embeddingBatchSize?: number;
  embeddingProcessCount?: number;
  embeddingTimeoutMs?: number;
  embedderType: 'local' | 'cloudflare';

  // MMR & Search
  mmrEnabled: boolean;
  mmrLambda?: number;
  mmrTokenBudget?: number;
  mmrDiversityMetric?: string;

  // Git & Telemetry
  includeUntracked: boolean;
  telemetryEnabled: boolean;
  telemetrySampleRate: number;
  telemetryAnonymization: string;
  telemetryRetentionDays: number;
}

/**
 * Get environment variable with CORTEX_ prefix preference
 * Falls back to unprefixed version for backward compatibility
 */
function getEnvVar(name: string, defaultValue?: string): string | undefined {
  const prefixed = process.env[`CORTEX_${name}`];
  const unprefixed = process.env[name];
  
  // Warn if using unprefixed version
  if (!prefixed && unprefixed) {
    console.warn(`[ENV] Using unprefixed environment variable '${name}'. Consider using 'CORTEX_${name}' to avoid conflicts.`);
  }
  
  return prefixed || unprefixed || defaultValue;
}

/**
 * Get boolean environment variable
 */
function getBooleanEnv(name: string, defaultValue: boolean = false): boolean {
  const value = getEnvVar(name);
  if (value === undefined) return defaultValue;
  return value.toLowerCase() === 'true';
}

/**
 * Get number environment variable
 */
function getNumberEnv(name: string, defaultValue?: number): number | undefined {
  const value = getEnvVar(name);
  if (value === undefined) return defaultValue;
  const parsed = parseInt(value, 10);
  return isNaN(parsed) ? defaultValue : parsed;
}

/**
 * Get float environment variable
 */
function getFloatEnv(name: string, defaultValue?: number): number | undefined {
  const value = getEnvVar(name);
  if (value === undefined) return defaultValue;
  const parsed = parseFloat(value);
  return isNaN(parsed) ? defaultValue : parsed;
}

/**
 * Load complete Cortex configuration from environment
 */
export function loadCortexConfig(): CortexConfig {
  return {
    // Core Configuration
    port: getNumberEnv('PORT', 8765)!,
    logFile: getEnvVar('LOG_FILE'),
    debug: getBooleanEnv('DEBUG', false),

    // Advanced Configuration
    disableRealTime: getBooleanEnv('DISABLE_REAL_TIME', false),
    enableNewLogging: getBooleanEnv('ENABLE_NEW_LOGGING', false),
    indexMode: getEnvVar('INDEX_MODE') as 'full' | 'incremental' | 'reindex' | undefined,
    forceRebuild: getBooleanEnv('FORCE_REBUILD', false),

    // Embedding & Processing
    embeddingStrategy: (getEnvVar('EMBEDDING_STRATEGY', 'auto') as 'auto' | 'cached' | 'process-pool'),
    embeddingBatchSize: getNumberEnv('EMBEDDING_BATCH_SIZE'),
    embeddingProcessCount: getNumberEnv('EMBEDDING_PROCESS_COUNT'),
    embeddingTimeoutMs: getNumberEnv('EMBEDDING_TIMEOUT_MS'),
    embedderType: (getEnvVar('EMBEDDER_TYPE', 'local') as 'local' | 'cloudflare'),

    // MMR & Search (these already use CORTEX_ prefix)
    mmrEnabled: getBooleanEnv('MMR_ENABLED', true),
    mmrLambda: getFloatEnv('MMR_LAMBDA'),
    mmrTokenBudget: getNumberEnv('MMR_TOKEN_BUDGET'),
    mmrDiversityMetric: getEnvVar('MMR_DIVERSITY_METRIC'),

    // Git & Telemetry (these already use CORTEX_ prefix)
    includeUntracked: getBooleanEnv('INCLUDE_UNTRACKED', false),
    telemetryEnabled: getBooleanEnv('TELEMETRY_ENABLED', true),
    telemetrySampleRate: getFloatEnv('TELEMETRY_SAMPLE_RATE', 1.0)!,
    telemetryAnonymization: getEnvVar('TELEMETRY_ANONYMIZATION', 'standard')!,
    telemetryRetentionDays: getNumberEnv('TELEMETRY_RETENTION_DAYS', 30)!,
  };
}

/**
 * Global configuration instance
 */
export const cortexConfig = loadCortexConfig();

/**
 * Legacy compatibility functions for existing code
 * @deprecated Use cortexConfig object instead
 */
export const legacyEnv = {
  PORT: cortexConfig.port.toString(),
  LOG_FILE: cortexConfig.logFile,
  DEBUG: cortexConfig.debug.toString(),
  DISABLE_REAL_TIME: cortexConfig.disableRealTime.toString(),
  ENABLE_NEW_LOGGING: cortexConfig.enableNewLogging.toString(),
  INDEX_MODE: cortexConfig.indexMode,
  FORCE_REBUILD: cortexConfig.forceRebuild.toString(),
};
