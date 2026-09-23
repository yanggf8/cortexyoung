/**
 * Enhanced Project Detector with Cross-File Analysis
 * Combines single-file and cross-file pattern detection for comprehensive project understanding
 */

import { CrossFilePatternDetector } from './cross-file-pattern-detector.js';
import { EnhancedProjectDetector } from './project-detector.js';
import type {
  ProjectContextInfo,
  ImplementationPatterns
} from '../types/patterns.js';
import type {
  CrossFileAnalysisResult,
  ArchitectureType
} from '../types/cross-file-analysis.js';

export interface EnhancedProjectContext extends ProjectContextInfo {
  /** Cross-file analysis results */
  crossFileAnalysis: CrossFileAnalysisResult;
  /** Architecture classification */
  architecture: ArchitectureType;
  /** Enhanced confidence scores */
  enhancedConfidence: {
    singleFileAccuracy: number;
    crossFileAccuracy: number;
    overallAccuracy: number;
  };
}

export class CrossFileEnhancedProjectDetector {
  private projectRoot: string;
  private singleFileDetector: EnhancedProjectDetector;
  private crossFileDetector: CrossFilePatternDetector;

  constructor(projectRoot: string = process.cwd()) {
    this.projectRoot = projectRoot;
    this.singleFileDetector = new EnhancedProjectDetector(projectRoot);
    this.crossFileDetector = new CrossFilePatternDetector(projectRoot);
  }

  /**
   * Main detection method, refocused on robust, single-file analysis for core accuracy.
   * This is the primary method for generating reliable context for Claude.
   */
  async detectProjectContext(): Promise<ProjectContextInfo> {
    console.log('🔍 Starting robust project context analysis...');
    
    // Use the reliable single-file detector as the primary source of truth.
    // This avoids the complexity and potential fragility of full cross-file analysis
    // for generating the core implementation patterns.
    const singleFileContext = this.singleFileDetector.detectCurrentContext();
    console.log('✅ Robust single-file analysis complete.');

    return {
      ...singleFileContext,
      lastUpdated: new Date().toISOString()
    };
  }

  /**
   * EXPERIMENTAL: Runs the full cross-file architectural analysis.
   * This is a heavy, complex operation intended for deep architectural insights,
   * not for generating the primary, most reliable implementation patterns for Claude.
   * The results are more experimental and should be treated with caution.
   */
  async runExperimentalCrossFileAnalysis(): Promise<EnhancedProjectContext> {
    console.log('🔬 Starting EXPERIMENTAL cross-file architectural analysis...');
    
    // Step 1: Traditional single-file detection
    const singleFileContext = this.singleFileDetector.detectCurrentContext();
    console.log('✅ Single-file analysis complete');

    // Step 2: Cross-file analysis  
    const crossFileAnalysis = await this.crossFileDetector.detectCrossFilePatterns();
    console.log('✅ Cross-file analysis complete');

    // Step 3: Merge and enhance patterns
    const enhancedPatterns = await this.mergeImplementationPatterns(
      singleFileContext.implementationPatterns,
      crossFileAnalysis
    );
    console.log('✅ Pattern integration complete');

    // Step 4: Calculate enhanced confidence
    const enhancedConfidence = this.calculateEnhancedConfidence(
      singleFileContext.implementationPatterns,
      crossFileAnalysis
    );

    const result: EnhancedProjectContext = {
      ...singleFileContext,
      implementationPatterns: enhancedPatterns,
      crossFileAnalysis,
      architecture: crossFileAnalysis.architecture,
      enhancedConfidence,
      lastUpdated: new Date().toISOString()
    };

    console.log(`🚀 EXPERIMENTAL analysis complete with ${enhancedConfidence.overallAccuracy}% accuracy`);
    return result;
  }

  /**
   * Quick detection using cached analysis.
   * Refocused to use the primary, robust detection method.
   */
  async detectWithCache(): Promise<ProjectContextInfo> {
    // In a real implementation, this would use a cache.
    // For now, it calls the main detection logic.
    return this.detectProjectContext();
  }

  /**
   * Focus on specific pattern types for faster analysis
   */
  async detectSpecificPatterns(patternTypes: ('authentication' | 'apiResponses' | 'errorHandling')[]): Promise<Partial<ImplementationPatterns>> {
    const patterns: Partial<ImplementationPatterns> = {};

    if (patternTypes.includes('authentication')) {
      const authPatterns = await this.crossFileDetector.detectAuthenticationPatterns();
      patterns.authentication = this.extractAuthenticationImplementation(authPatterns);
    }

    if (patternTypes.includes('apiResponses')) {
      const apiPatterns = await this.crossFileDetector.detectAPIResponsePatterns();
      patterns.apiResponses = this.extractAPIResponseImplementation(apiPatterns);
    }

    if (patternTypes.includes('errorHandling')) {
      // Error handling would be detected through execution flow analysis
      const fullAnalysis = await this.crossFileDetector.detectCrossFilePatterns();
      patterns.errorHandling = this.extractErrorHandlingImplementation(fullAnalysis);
    }

    return patterns;
  }

  /**
   * Get architectural insights from cross-file analysis
   */
  async getArchitecturalInsights(): Promise<{
    architecture: ArchitectureType;
    layerCount: number;
    componentCount: number;
    complexity: 'low' | 'medium' | 'high';
    recommendations: string[];
  }> {
    const analysis = await this.crossFileDetector.detectCrossFilePatterns();
    
    const layerCount = this.countArchitecturalLayers(analysis);
    const componentCount = analysis.dependencyGraph.files.size;
    const complexity = this.assessComplexity(analysis);
    const recommendations = this.generateRecommendations(analysis);

    return {
      architecture: analysis.architecture,
      layerCount,
      componentCount,
      complexity,
      recommendations
    };
  }

  private async mergeImplementationPatterns(
    singleFilePatterns: ImplementationPatterns,
    crossFileAnalysis: CrossFileAnalysisResult
  ): Promise<ImplementationPatterns> {
    // Generate cross-file patterns
    const crossFilePatterns = await this.crossFileDetector.generateImplementationPatterns();

    // Merge authentication patterns
    const mergedAuth = this.mergeAuthenticationPatterns(
      singleFilePatterns.authentication,
      crossFilePatterns.authentication
    );

    // Merge API response patterns
    const mergedAPI = this.mergeAPIResponsePatterns(
      singleFilePatterns.apiResponses,
      crossFilePatterns.apiResponses
    );

    // Merge error handling patterns
    const mergedError = this.mergeErrorHandlingPatterns(
      singleFilePatterns.errorHandling,
      crossFilePatterns.errorHandling
    );

    return {
      authentication: mergedAuth,
      apiResponses: mergedAPI,
      errorHandling: mergedError
    };
  }

  private mergeAuthenticationPatterns(singleFile: any, crossFile: any): any {
    return {
      userProperty: this.selectBestValue(singleFile.userProperty, crossFile.userProperty, singleFile.confidence, crossFile.confidence),
      tokenLocation: this.selectBestValue(singleFile.tokenLocation, crossFile.tokenLocation, singleFile.confidence, crossFile.confidence),
      errorResponse: {
        format: this.selectBestValue(singleFile.errorResponse.format, crossFile.errorResponse.format, singleFile.confidence, crossFile.confidence),
        statusCode: this.selectBestValue(singleFile.errorResponse.statusCode, crossFile.errorResponse.statusCode, singleFile.confidence, crossFile.confidence)
      },
      middlewarePattern: this.selectBestValue(singleFile.middlewarePattern, crossFile.middlewarePattern, singleFile.confidence, crossFile.confidence),
      confidence: Math.max(singleFile.confidence, crossFile.confidence),
      evidence: [...(singleFile.evidence || []), ...(crossFile.evidence || [])],
      crossFileEvidence: crossFile.evidence || []
    };
  }

  private mergeAPIResponsePatterns(singleFile: any, crossFile: any): any {
    return {
      successFormat: this.selectBestValue(singleFile.successFormat, crossFile.successFormat, singleFile.confidence, crossFile.confidence),
      errorFormat: this.selectBestValue(singleFile.errorFormat, crossFile.errorFormat, singleFile.confidence, crossFile.confidence),
      statusCodeUsage: this.selectBestValue(singleFile.statusCodeUsage, crossFile.statusCodeUsage, singleFile.confidence, crossFile.confidence),
      wrapperPattern: this.selectBestValue(singleFile.wrapperPattern, crossFile.wrapperPattern, singleFile.confidence, crossFile.confidence),
      confidence: Math.max(singleFile.confidence, crossFile.confidence),
      evidence: [...(singleFile.evidence || []), ...(crossFile.evidence || [])],
      crossFileEvidence: crossFile.evidence || []
    };
  }

  private mergeErrorHandlingPatterns(singleFile: any, crossFile: any): any {
    return {
      catchPattern: this.selectBestValue(singleFile.catchPattern, crossFile.catchPattern, singleFile.confidence, crossFile.confidence),
      errorStructure: this.selectBestValue(singleFile.errorStructure, crossFile.errorStructure, singleFile.confidence, crossFile.confidence),
      loggingIntegration: this.selectBestValue(singleFile.loggingIntegration, crossFile.loggingIntegration, singleFile.confidence, crossFile.confidence),
      propagationStyle: this.selectBestValue(singleFile.propagationStyle, crossFile.propagationStyle, singleFile.confidence, crossFile.confidence),
      confidence: Math.max(singleFile.confidence, crossFile.confidence),
      evidence: [...(singleFile.evidence || []), ...(crossFile.evidence || [])],
      crossFileEvidence: crossFile.evidence || []
    };
  }

  private selectBestValue(singleValue: any, crossValue: any, singleConfidence: number, crossConfidence: number): any {
    // If one is unknown and the other isn't, prefer the known value
    if (singleValue === 'Unknown' && crossValue !== 'Unknown') return crossValue;
    if (crossValue === 'Unknown' && singleValue !== 'Unknown') return singleValue;

    // If both are known, prefer the one with higher confidence
    if (crossConfidence > singleConfidence) return crossValue;
    return singleValue;
  }

  private calculateEnhancedConfidence(singleFilePatterns: ImplementationPatterns, crossFileAnalysis: CrossFileAnalysisResult): any {
    const singleFileAccuracy = Math.round((
      singleFilePatterns.authentication.confidence +
      singleFilePatterns.apiResponses.confidence +
      singleFilePatterns.errorHandling.confidence
    ) / 3);

    const crossFileAccuracy = crossFileAnalysis.metadata.overallConfidence;

    // Enhanced accuracy is weighted average, favoring cross-file analysis
    const overallAccuracy = Math.round(
      (singleFileAccuracy * 0.3) + (crossFileAccuracy * 0.7)
    );

    return {
      singleFileAccuracy,
      crossFileAccuracy,
      overallAccuracy
    };
  }

  private extractAuthenticationImplementation(patterns: any[]): any {
    if (patterns.length === 0) {
      return {
        userProperty: 'Unknown',
        tokenLocation: 'Unknown',
        errorResponse: { format: 'Unknown', statusCode: 0 },
        middlewarePattern: 'Unknown',
        confidence: 0,
        evidence: [],
        crossFileEvidence: []
      };
    }

    const pattern = patterns[0];
    return {
      userProperty: this.extractUserPropertyFromPattern(pattern),
      tokenLocation: this.extractTokenLocationFromPattern(pattern),
      errorResponse: {
        format: '{message: string}',
        statusCode: 401
      },
      middlewarePattern: 'cross-file authentication flow',
      confidence: pattern.confidence,
      evidence: pattern.evidence.map((e: any) => `${e.file}:${e.lineNumber} - ${e.snippet}`),
      crossFileEvidence: pattern.evidence
    };
  }

  private extractAPIResponseImplementation(patterns: any[]): any {
    if (patterns.length === 0) {
      return {
        successFormat: 'Unknown',
        errorFormat: 'Unknown',
        statusCodeUsage: 'Unknown',
        wrapperPattern: 'Unknown',
        confidence: 0,
        evidence: [],
        crossFileEvidence: []
      };
    }

    const pattern = patterns[0];
    return {
      successFormat: 'cross-file response handling',
      errorFormat: '{message: string}',
      statusCodeUsage: 'explicit codes',
      wrapperPattern: 'conditional',
      confidence: pattern.confidence,
      evidence: pattern.evidence.map((e: any) => `${e.file}:${e.lineNumber} - ${e.snippet}`),
      crossFileEvidence: pattern.evidence
    };
  }

  private extractErrorHandlingImplementation(analysis: CrossFileAnalysisResult): any {
    const errorPatterns = analysis.patterns.filter(p => 
      p.type === 'error_propagation' || 
      p.description.toLowerCase().includes('error')
    );

    if (errorPatterns.length === 0) {
      return {
        catchPattern: 'Unknown',
        errorStructure: 'Unknown',
        loggingIntegration: 'Unknown',
        propagationStyle: 'Unknown',
        confidence: 0,
        evidence: [],
        crossFileEvidence: []
      };
    }

    const pattern = errorPatterns[0];
    return {
      catchPattern: 'cross-file error handling',
      errorStructure: '{message: string}',
      loggingIntegration: 'integrated',
      propagationStyle: 'throw exceptions',
      confidence: pattern.confidence,
      evidence: pattern.evidence.map((e: any) => `${e.file}:${e.lineNumber} - ${e.snippet}`),
      crossFileEvidence: pattern.evidence
    };
  }

  private extractUserPropertyFromPattern(pattern: any): string {
    for (const evidence of pattern.evidence) {
      if (evidence.snippet.includes('req.user')) return 'req.user';
      if (evidence.snippet.includes('req.auth')) return 'req.auth';
      if (evidence.snippet.includes('req.context.user')) return 'req.context.user';
    }
    return 'Unknown';
  }

  private extractTokenLocationFromPattern(pattern: any): string {
    for (const evidence of pattern.evidence) {
      if (evidence.snippet.toLowerCase().includes('cookie')) return 'httpOnly cookie';
      if (evidence.snippet.toLowerCase().includes('header') || evidence.snippet.toLowerCase().includes('bearer')) {
        return 'authorization header';
      }
    }
    return 'Unknown';
  }

  private countArchitecturalLayers(analysis: CrossFileAnalysisResult): number {
    const layerTypes = new Set<string>();
    
    for (const pattern of analysis.patterns) {
      for (const file of pattern.files) {
        const fileName = file.toLowerCase();
        if (fileName.includes('route') || fileName.includes('controller')) layerTypes.add('presentation');
        if (fileName.includes('service')) layerTypes.add('business');
        if (fileName.includes('model') || fileName.includes('repository')) layerTypes.add('data');
        if (fileName.includes('middleware')) layerTypes.add('infrastructure');
      }
    }

    return layerTypes.size;
  }

  private assessComplexity(analysis: CrossFileAnalysisResult): 'low' | 'medium' | 'high' {
    const fileCount = analysis.dependencyGraph.files.size;
    const patternCount = analysis.patterns.length;
    const avgPathLength = analysis.executionFlows.length > 0 
      ? analysis.executionFlows.reduce((sum, flow) => sum + flow.callChain.length, 0) / analysis.executionFlows.length 
      : 0;

    const complexityScore = fileCount * 0.1 + patternCount * 2 + avgPathLength;

    if (complexityScore < 10) return 'low';
    if (complexityScore < 25) return 'medium';
    return 'high';
  }

  private generateRecommendations(analysis: CrossFileAnalysisResult): string[] {
    const recommendations: string[] = [];

    // Based on architecture type
    switch (analysis.architecture) {
      case 'mvc':
        recommendations.push('Consider implementing proper separation of concerns between Model, View, and Controller');
        break;
      case 'layered':
        recommendations.push('Maintain clear boundaries between architectural layers');
        break;
      case 'mixed':
        recommendations.push('Consider refactoring to a more consistent architectural pattern');
        break;
    }

    // Based on complexity
    const fileCount = analysis.dependencyGraph.files.size;
    if (fileCount > 50) {
      recommendations.push('Consider breaking down large modules into smaller, focused components');
    }

    // Based on patterns detected
    const hasAuthFlow = analysis.patterns.some(p => p.type === 'auth_flow');
    if (!hasAuthFlow && fileCount > 5) {
      recommendations.push('Consider implementing consistent authentication flow patterns');
    }

    return recommendations;
  }
}