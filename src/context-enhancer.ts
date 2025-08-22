import { ProjectContextDetector, ProjectContext } from './project-context-detector';
import { log, warn } from './logging-utils';

interface ContextEnhancementOptions {
  maxTokens?: number;
  includePatterns?: boolean;
  includeDirectories?: boolean;
  includeDependencies?: boolean;
}

interface EnhancementStats {
  enhanced: boolean;
  tokensAdded: number;
  contextType: string;
  confidence: number;
}

/**
 * Context Enhancement Layer for Cortex V3.0
 * 
 * Enhances semantic search results with essential project context to improve
 * Claude Code's project awareness and suggestion accuracy.
 * 
 * Strategy: Prepend structured project information to search results
 * - Simple, focused approach (reject query-specific complexity)
 * - Token budget aware (<150 tokens max)
 * - Essential context only (type, structure, libraries)
 */
export class ContextEnhancer {
  private detector: ProjectContextDetector;
  private enhancementCount = 0;
  private enhancementStats: Map<string, number> = new Map();

  constructor() {
    this.detector = new ProjectContextDetector();
  }

  /**
   * Enhance semantic search results with project context
   * Core V3.0 functionality - prepends essential project info
   */
  async enhanceSemanticResults(
    semanticResults: string,
    query: string,
    projectPath: string,
    options: ContextEnhancementOptions = {}
  ): Promise<{ results: string; stats: EnhancementStats }> {
    
    const opts = {
      maxTokens: 150,
      includePatterns: true,
      includeDirectories: true,
      includeDependencies: true,
      ...options
    };

    try {
      // Detect project context
      const projectContext = await this.detector.detectProjectContext(projectPath);
      
      // Skip enhancement for unknown projects or low confidence
      if (projectContext.type === 'Unknown Project' || projectContext.confidence < 0.5) {
        return {
          results: semanticResults,
          stats: {
            enhanced: false,
            tokensAdded: 0,
            contextType: 'unknown',
            confidence: projectContext.confidence
          }
        };
      }

      // Generate context header
      const contextHeader = this.formatContextHeader(projectContext, opts);
      
      // Check token budget
      const estimatedTokens = this.estimateTokens(contextHeader);
      if (estimatedTokens > opts.maxTokens) {
        // Try minimal context
        const minimalHeader = this.formatMinimalContext(projectContext);
        const minimalTokens = this.estimateTokens(minimalHeader);
        
        if (minimalTokens <= opts.maxTokens) {
          const enhancedResults = `${minimalHeader}\n\n${semanticResults}`;
          this.trackEnhancement(projectContext.type, minimalTokens);
          
          return {
            results: enhancedResults,
            stats: {
              enhanced: true,
              tokensAdded: minimalTokens,
              contextType: 'minimal',
              confidence: projectContext.confidence
            }
          };
        }
        
        // Skip enhancement if even minimal context exceeds budget
        return {
          results: semanticResults,
          stats: {
            enhanced: false,
            tokensAdded: 0,
            contextType: 'budget_exceeded',
            confidence: projectContext.confidence
          }
        };
      }

      // Apply full context enhancement
      const enhancedResults = `${contextHeader}\n\n${semanticResults}`;
      this.trackEnhancement(projectContext.type, estimatedTokens);

      return {
        results: enhancedResults,
        stats: {
          enhanced: true,
          tokensAdded: estimatedTokens,
          contextType: 'full',
          confidence: projectContext.confidence
        }
      };

    } catch (error) {
      warn('Context enhancement failed', { projectPath, query, error });
      
      return {
        results: semanticResults,
        stats: {
          enhanced: false,
          tokensAdded: 0,
          contextType: 'error',
          confidence: 0
        }
      };
    }
  }

  /**
   * Format complete context header with all available information
   */
  private formatContextHeader(context: ProjectContext, options: ContextEnhancementOptions): string {
    const parts: string[] = [];

    // Core project identification (always included)
    parts.push(`PROJECT: ${context.type}`);
    
    // Language and framework info
    if (context.language !== 'unknown') {
      parts.push(`LANGUAGE: ${context.language}`);
    }
    
    if (context.framework !== 'unknown' && context.framework !== context.language) {
      parts.push(`FRAMEWORK: ${context.framework}`);
    }

    // Directory structure (if enabled and available)
    if (options.includeDirectories && context.directories.length > 0) {
      parts.push(`STRUCTURE: ${context.directories.join(', ')}`);
    }

    // Core dependencies (if enabled and available)  
    if (options.includeDependencies && context.dependencies.length > 0) {
      parts.push(`LIBRARIES: ${context.dependencies.join(', ')}`);
    }

    // Key patterns (if enabled and available)
    if (options.includePatterns && Object.keys(context.patterns).length > 0) {
      const patternStrings = Object.entries(context.patterns)
        .filter(([_, value]) => value)
        .map(([key, value]) => `${key}=${value}`)
        .slice(0, 2); // Limit to 2 patterns to control tokens
      
      if (patternStrings.length > 0) {
        parts.push(`PATTERNS: ${patternStrings.join(', ')}`);
      }
    }

    return parts.join('\n');
  }

  /**
   * Format minimal context header (fallback for token budget constraints)
   */
  private formatMinimalContext(context: ProjectContext): string {
    const parts: string[] = [];
    
    parts.push(`PROJECT: ${context.type}`);
    
    // Only include most essential info
    if (context.dependencies.length > 0) {
      const topDeps = context.dependencies.slice(0, 3); // Top 3 dependencies only
      parts.push(`LIBRARIES: ${topDeps.join(', ')}`);
    }

    return parts.join('\n');
  }

  /**
   * Estimate token count for context header
   * Simple approximation: ~4 characters per token
   */
  private estimateTokens(text: string): number {
    return Math.ceil(text.length / 4);
  }

  /**
   * Track enhancement statistics for monitoring
   */
  private trackEnhancement(projectType: string, tokens: number): void {
    this.enhancementCount++;
    const currentCount = this.enhancementStats.get(projectType) || 0;
    this.enhancementStats.set(projectType, currentCount + 1);
    
    log('Context enhancement applied', {
      projectType,
      tokensAdded: tokens,
      totalEnhancements: this.enhancementCount
    });
  }

  /**
   * Get enhancement statistics for monitoring
   */
  getEnhancementStats(): {
    totalEnhancements: number;
    byProjectType: Record<string, number>;
    cacheStats: { entries: number; hitRate: number };
  } {
    return {
      totalEnhancements: this.enhancementCount,
      byProjectType: Object.fromEntries(this.enhancementStats.entries()),
      cacheStats: this.detector.getCacheStats()
    };
  }

  /**
   * Invalidate project context cache (for file watching integration)
   */
  invalidateProjectCache(projectPath: string): void {
    this.detector.invalidateCache(projectPath);
  }

  /**
   * Clear all caches
   */
  clearCaches(): void {
    this.detector.clearCache();
    log('Context enhancer caches cleared');
  }

  /**
   * Test context enhancement for a specific project
   * Useful for validation and debugging
   */
  async testEnhancement(projectPath: string): Promise<{
    context: ProjectContext;
    fullHeader: string;
    minimalHeader: string;
    tokenEstimates: {
      full: number;
      minimal: number;
    };
  }> {
    const context = await this.detector.detectProjectContext(projectPath);
    
    const fullHeader = this.formatContextHeader(context, {
      maxTokens: 150,
      includePatterns: true,
      includeDirectories: true,
      includeDependencies: true
    });
    
    const minimalHeader = this.formatMinimalContext(context);
    
    return {
      context,
      fullHeader,
      minimalHeader,
      tokenEstimates: {
        full: this.estimateTokens(fullHeader),
        minimal: this.estimateTokens(minimalHeader)
      }
    };
  }
}