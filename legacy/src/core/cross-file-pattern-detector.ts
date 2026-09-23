/**
 * Cross-File Pattern Detection Engine
 * Main orchestrator for detecting architectural patterns across multiple files
 */

import { DependencyGraphBuilder } from './dependency-graph-builder.js';
import { SymbolResolver } from './symbol-resolver.js';
import { ExecutionFlowTracer } from './execution-flow-tracer.js';
import type {
  CrossFileAnalysisResult,
  CrossFilePattern,
  CrossFilePatternType,
  CrossFileEvidence,
  EvidenceType,
  ArchitectureType,
  AnalysisMetadata
} from '../types/cross-file-analysis.js';
import type {
  ImplementationPatterns
} from '../types/patterns.js';

export class CrossFilePatternDetector {
  private projectRoot: string;
  private graphBuilder: DependencyGraphBuilder;
  private symbolResolver: SymbolResolver | null = null;
  private flowTracer: ExecutionFlowTracer | null = null;

  constructor(projectRoot: string = process.cwd()) {
    this.projectRoot = projectRoot;
    this.graphBuilder = new DependencyGraphBuilder(projectRoot);
  }

  /**
   * Main detection method - performs complete cross-file analysis
   */
  async detectCrossFilePatterns(): Promise<CrossFileAnalysisResult> {
    const startTime = Date.now();
    console.log('🚀 Starting cross-file pattern detection...');

    // Step 1: Build dependency graph
    const dependencyGraph = await this.graphBuilder.buildGraph();
    console.log('📊 Dependency graph constructed');

    // Step 2: Resolve symbols across files
    this.symbolResolver = new SymbolResolver(dependencyGraph);
    const symbolResolution = this.symbolResolver.resolveAllSymbols();
    console.log(`🎯 Symbol resolution: ${symbolResolution.stats.resolutionAccuracy}% accuracy`);

    // Step 3: Trace execution flows
    this.flowTracer = new ExecutionFlowTracer(dependencyGraph, this.symbolResolver);
    const flowResult = this.flowTracer.traceExecutionFlows();
    console.log(`🛤️ Execution flow analysis: ${flowResult.executionPaths.length} paths traced`);

    // Step 4: Detect cross-file patterns
    const patterns = this.detectPatterns(dependencyGraph, symbolResolution, flowResult);
    console.log(`🔍 Detected ${patterns.length} cross-file patterns`);

    // Step 5: Classify overall architecture
    const architecture = this.classifyArchitecture(patterns, flowResult);
    console.log(`🏛️ Architecture classified as: ${architecture}`);

    const executionTime = Date.now() - startTime;

    const result: CrossFileAnalysisResult = {
      dependencyGraph,
      patterns,
      executionFlows: flowResult.executionPaths,
      architecture,
      metadata: {
        timestamp: new Date().toISOString(),
        filesAnalyzed: dependencyGraph.files.size,
        executionTimeMs: executionTime,
        depthLevel: this.calculateAnalysisDepth(patterns),
        overallConfidence: this.calculateOverallConfidence(patterns)
      }
    };

    console.log(`✅ Cross-file analysis complete in ${executionTime}ms`);
    return result;
  }

  /**
   * Detect specific cross-file authentication patterns
   */
  async detectAuthenticationPatterns(): Promise<CrossFilePattern[]> {
    if (!this.flowTracer) {
      await this.initializeComponents();
    }

    const authFlows = this.flowTracer!.traceAuthenticationFlows();
    const patterns: CrossFilePattern[] = [];

    // Group flows by pattern type
    const groupedFlows = this.groupFlowsByPattern(authFlows);

    for (const [patternType, flows] of groupedFlows) {
      if (patternType === 'auth_flow') {
        const pattern = this.createAuthenticationPattern(flows);
        if (pattern) {
          patterns.push(pattern);
        }
      }
    }

    return patterns;
  }

  /**
   * Detect API response patterns across files
   */
  async detectAPIResponsePatterns(): Promise<CrossFilePattern[]> {
    if (!this.flowTracer) {
      await this.initializeComponents();
    }

    const apiFlows = this.flowTracer!.traceAPIResponseFlows();
    const patterns: CrossFilePattern[] = [];

    const groupedFlows = this.groupFlowsByPattern(apiFlows);

    for (const [patternType, flows] of groupedFlows) {
      if (patternType === 'api_response_flow') {
        const pattern = this.createAPIResponsePattern(flows);
        if (pattern) {
          patterns.push(pattern);
        }
      }
    }

    return patterns;
  }

  /**
   * Detect MVC architectural patterns
   */
  async detectMVCPatterns(): Promise<CrossFilePattern[]> {
    if (!this.flowTracer) {
      await this.initializeComponents();
    }

    const mvcFlows = this.flowTracer!.traceMVCFlows();
    const patterns: CrossFilePattern[] = [];

    const groupedFlows = this.groupFlowsByPattern(mvcFlows);

    for (const [patternType, flows] of groupedFlows) {
      if (patternType === 'mvc_flow') {
        const pattern = this.createMVCPattern(flows);
        if (pattern) {
          patterns.push(pattern);
        }
      }
    }

    return patterns;
  }

  /**
   * Derives implementation patterns from the results of a full cross-file analysis.
   * WARNING: This is an indirect method. The generated patterns are inferred from
   * architectural analysis and may be less accurate than direct pattern detection.
   * This is kept for experimental and comparison purposes.
   */
  async generateImplementationPatterns(): Promise<ImplementationPatterns> {
    console.warn(
      `[ClaudeCat] WARNING: Generating implementation patterns from cross-file analysis. This is an indirect, experimental method and may be less accurate than direct detection.`
    );
    const crossFileResult = await this.detectCrossFilePatterns();
    
    // Extract authentication patterns from cross-file analysis
    const authPatterns = crossFileResult.patterns.filter(p => p.type === 'auth_flow');
    const authEvidence = authPatterns.flatMap(p => p.evidence);
    
    // Extract API response patterns
    const apiPatterns = crossFileResult.patterns.filter(p => p.type === 'api_response_flow');
    const apiEvidence = apiPatterns.flatMap(p => p.evidence);
    
    // Extract error handling patterns
    const errorPatterns = crossFileResult.patterns.filter(p => p.type === 'error_propagation');
    const errorEvidence = errorPatterns.flatMap(p => p.evidence);

    return {
      authentication: this.buildAuthImplementation(authEvidence),
      apiResponses: this.buildAPIImplementation(apiEvidence),
      errorHandling: this.buildErrorImplementation(errorEvidence)
    };
  }

  private async initializeComponents(): Promise<void> {
    if (!this.symbolResolver || !this.flowTracer) {
      const dependencyGraph = await this.graphBuilder.buildGraph();
      this.symbolResolver = new SymbolResolver(dependencyGraph);
      this.symbolResolver.resolveAllSymbols();
      this.flowTracer = new ExecutionFlowTracer(dependencyGraph, this.symbolResolver);
    }
  }

  private detectPatterns(dependencyGraph: any, symbolResolution: any, flowResult: any): CrossFilePattern[] {
    const patterns: CrossFilePattern[] = [];

    // Detect authentication flows
    const authFlows = flowResult.executionPaths.filter((p: any) => p.patternType === 'auth_flow');
    if (authFlows.length > 0) {
      const authPattern = this.createAuthenticationPattern(authFlows);
      if (authPattern) patterns.push(authPattern);
    }

    // Detect API response flows
    const apiFlows = flowResult.executionPaths.filter((p: any) => p.patternType === 'api_response_flow');
    if (apiFlows.length > 0) {
      const apiPattern = this.createAPIResponsePattern(apiFlows);
      if (apiPattern) patterns.push(apiPattern);
    }

    // Detect MVC flows
    const mvcFlows = flowResult.executionPaths.filter((p: any) => p.patternType === 'mvc_flow');
    if (mvcFlows.length > 0) {
      const mvcPattern = this.createMVCPattern(mvcFlows);
      if (mvcPattern) patterns.push(mvcPattern);
    }

    // Detect service composition patterns
    const serviceFlows = flowResult.executionPaths.filter((p: any) => p.patternType === 'service_composition');
    if (serviceFlows.length > 0) {
      const servicePattern = this.createServiceCompositionPattern(serviceFlows);
      if (servicePattern) patterns.push(servicePattern);
    }

    // Detect middleware chains
    const middlewareFlows = flowResult.executionPaths.filter((p: any) => p.patternType === 'middleware_chain');
    if (middlewareFlows.length > 0) {
      const middlewarePattern = this.createMiddlewareChainPattern(middlewareFlows);
      if (middlewarePattern) patterns.push(middlewarePattern);
    }

    // Detect layered architecture
    const layeredFlows = flowResult.executionPaths.filter((p: any) => p.patternType === 'layered_architecture');
    if (layeredFlows.length > 0) {
      const layeredPattern = this.createLayeredArchitecturePattern(layeredFlows);
      if (layeredPattern) patterns.push(layeredPattern);
    }

    return patterns;
  }

  private createAuthenticationPattern(flows: any[]): CrossFilePattern | null {
    if (flows.length === 0) return null;

    const allFiles = [...new Set(flows.flatMap((f: any) => f.filesInvolved))];
    const evidence: CrossFileEvidence[] = [];

    // Analyze authentication evidence across files
    for (const flow of flows) {
      for (const step of flow.callChain) {
        if (this.isAuthenticationStep(step)) {
          evidence.push({
            type: 'middleware_usage',
            file: step.file,
            lineNumber: step.lineNumber,
            snippet: `${step.function}() - Authentication middleware`,
            weight: 30
          });
        }
      }
    }

    const confidence = this.calculatePatternConfidence(evidence);

    return {
      id: 'auth_flow_pattern',
      type: 'auth_flow',
      files: allFiles,
      paths: flows,
      confidence,
      evidence,
      description: `Authentication flow spanning ${allFiles.length} files with ${flows.length} execution paths`
    };
  }

  private createAPIResponsePattern(flows: any[]): CrossFilePattern | null {
    if (flows.length === 0) return null;

    const allFiles = [...new Set(flows.flatMap((f: any) => f.filesInvolved))];
    const evidence: CrossFileEvidence[] = [];

    for (const flow of flows) {
      for (const step of flow.callChain) {
        if (this.isAPIResponseStep(step)) {
          evidence.push({
            type: 'function_call',
            file: step.file,
            lineNumber: step.lineNumber,
            snippet: `${step.function}() - API response handler`,
            weight: 25
          });
        }
      }
    }

    const confidence = this.calculatePatternConfidence(evidence);

    return {
      id: 'api_response_pattern',
      type: 'api_response_flow',
      files: allFiles,
      paths: flows,
      confidence,
      evidence,
      description: `API response flow across ${allFiles.length} files with standardized response handling`
    };
  }

  private createMVCPattern(flows: any[]): CrossFilePattern | null {
    if (flows.length === 0) return null;

    const allFiles = [...new Set(flows.flatMap((f: any) => f.filesInvolved))];
    const evidence: CrossFileEvidence[] = [];

    // Check for MVC structure
    const hasRoute = flows.some((f: any) => f.filesInvolved.some((file: string) => this.isRouteFile(file)));
    const hasController = flows.some((f: any) => f.filesInvolved.some((file: string) => this.isControllerFile(file)));
    const hasModel = flows.some((f: any) => f.filesInvolved.some((file: string) => this.isModelFile(file)));

    if (hasRoute && hasController) {
      evidence.push({
        type: 'route_definition',
        file: allFiles.find(f => this.isRouteFile(f)) || allFiles[0],
        lineNumber: 0,
        snippet: 'Route -> Controller pattern detected',
        weight: 40
      });
    }

    if (hasController && hasModel) {
      evidence.push({
        type: 'function_call',
        file: allFiles.find(f => this.isControllerFile(f)) || allFiles[0],
        lineNumber: 0,
        snippet: 'Controller -> Model pattern detected',
        weight: 35
      });
    }

    const confidence = this.calculatePatternConfidence(evidence);

    return {
      id: 'mvc_pattern',
      type: 'mvc_flow',
      files: allFiles,
      paths: flows,
      confidence,
      evidence,
      description: `MVC architectural pattern with ${allFiles.length} files following Model-View-Controller structure`
    };
  }

  private createServiceCompositionPattern(flows: any[]): CrossFilePattern | null {
    if (flows.length === 0) return null;

    const allFiles = [...new Set(flows.flatMap((f: any) => f.filesInvolved))];
    const evidence: CrossFileEvidence[] = [];

    const serviceFiles = allFiles.filter(f => this.isServiceFile(f));
    
    if (serviceFiles.length >= 2) {
      evidence.push({
        type: 'function_call',
        file: serviceFiles[0],
        lineNumber: 0,
        snippet: `Service composition across ${serviceFiles.length} services`,
        weight: 35
      });
    }

    const confidence = this.calculatePatternConfidence(evidence);

    return {
      id: 'service_composition',
      type: 'service_composition',
      files: allFiles,
      paths: flows,
      confidence,
      evidence,
      description: `Service composition pattern with ${serviceFiles.length} interconnected services`
    };
  }

  private createMiddlewareChainPattern(flows: any[]): CrossFilePattern | null {
    if (flows.length === 0) return null;

    const allFiles = [...new Set(flows.flatMap((f: any) => f.filesInvolved))];
    const evidence: CrossFileEvidence[] = [];

    const middlewareFiles = allFiles.filter(f => this.isMiddlewareFile(f));
    
    if (middlewareFiles.length >= 2) {
      evidence.push({
        type: 'middleware_usage',
        file: middlewareFiles[0],
        lineNumber: 0,
        snippet: `Middleware chain with ${middlewareFiles.length} middleware components`,
        weight: 30
      });
    }

    const confidence = this.calculatePatternConfidence(evidence);

    return {
      id: 'middleware_chain',
      type: 'middleware_chain',
      files: allFiles,
      paths: flows,
      confidence,
      evidence,
      description: `Middleware chain pattern with ${middlewareFiles.length} middleware components`
    };
  }

  private createLayeredArchitecturePattern(flows: any[]): CrossFilePattern | null {
    if (flows.length === 0) return null;

    const allFiles = [...new Set(flows.flatMap((f: any) => f.filesInvolved))];
    const evidence: CrossFileEvidence[] = [];

    const layers = this.identifyArchitecturalLayers(allFiles);
    
    if (layers.size >= 3) {
      evidence.push({
        type: 'function_call',
        file: allFiles[0],
        lineNumber: 0,
        snippet: `Layered architecture with ${layers.size} distinct layers`,
        weight: 40
      });
    }

    const confidence = this.calculatePatternConfidence(evidence);

    return {
      id: 'layered_architecture',
      type: 'layered_architecture',
      files: allFiles,
      paths: flows,
      confidence,
      evidence,
      description: `Layered architecture pattern with ${layers.size} architectural layers`
    };
  }

  private classifyArchitecture(patterns: CrossFilePattern[], flowResult: any): ArchitectureType {
    const patternTypes = patterns.map(p => p.type);
    
    // Count pattern frequencies
    const patternCounts = new Map<CrossFilePatternType, number>();
    for (const type of patternTypes) {
      patternCounts.set(type, (patternCounts.get(type) || 0) + 1);
    }

    // Determine architecture based on dominant patterns
    if (patternCounts.has('mvc_flow') && patternCounts.get('mvc_flow')! > 0) {
      return 'mvc';
    }
    
    if (patternCounts.has('layered_architecture') && patternCounts.get('layered_architecture')! > 0) {
      return 'layered';
    }
    
    if (patternCounts.has('service_composition') && patternCounts.get('service_composition')! > 1) {
      return 'microservices';
    }
    
    if (patternCounts.has('middleware_chain') && patternCounts.get('middleware_chain')! > 0) {
      return 'component_based';
    }

    // Mixed if multiple different patterns
    if (patternCounts.size >= 3) {
      return 'mixed';
    }

    // Default classification based on file count
    const totalFiles = flowResult.stats?.totalPaths || 0;
    if (totalFiles > 20) {
      return 'monolithic';
    }

    return 'unknown';
  }

  private groupFlowsByPattern(flows: any[]): Map<CrossFilePatternType, any[]> {
    const groups = new Map<CrossFilePatternType, any[]>();
    
    for (const flow of flows) {
      const existing = groups.get(flow.patternType) || [];
      existing.push(flow);
      groups.set(flow.patternType, existing);
    }
    
    return groups;
  }

  private calculatePatternConfidence(evidence: CrossFileEvidence[]): number {
    if (evidence.length === 0) return 0;
    
    const totalWeight = evidence.reduce((sum, e) => sum + e.weight, 0);
    const maxPossibleWeight = evidence.length * 40; // Assuming max weight is 40
    
    return Math.min(100, Math.round((totalWeight / maxPossibleWeight) * 100));
  }

  private calculateAnalysisDepth(patterns: CrossFilePattern[]): number {
    if (patterns.length === 0) return 0;
    
    const avgFilesPerPattern = patterns.reduce((sum, p) => sum + p.files.length, 0) / patterns.length;
    return Math.round(avgFilesPerPattern);
  }

  private calculateOverallConfidence(patterns: CrossFilePattern[]): number {
    if (patterns.length === 0) return 0;
    
    const avgConfidence = patterns.reduce((sum, p) => sum + p.confidence, 0) / patterns.length;
    return Math.round(avgConfidence);
  }

  private isAuthenticationStep(step: any): boolean {
    const func = step.function.toLowerCase();
    return func.includes('auth') || func.includes('verify') || func.includes('check') || func.includes('validate');
  }

  private isAPIResponseStep(step: any): boolean {
    const func = step.function.toLowerCase();
    return func.includes('response') || func.includes('send') || func.includes('json') || func.includes('reply');
  }

  private isRouteFile(filePath: string): boolean {
    return filePath.toLowerCase().includes('route');
  }

  private isControllerFile(filePath: string): boolean {
    return filePath.toLowerCase().includes('controller');
  }

  private isModelFile(filePath: string): boolean {
    return filePath.toLowerCase().includes('model');
  }

  private isServiceFile(filePath: string): boolean {
    return filePath.toLowerCase().includes('service');
  }

  private isMiddlewareFile(filePath: string): boolean {
    return filePath.toLowerCase().includes('middleware');
  }

  private identifyArchitecturalLayers(files: string[]): Set<string> {
    const layers = new Set<string>();
    
    for (const file of files) {
      const fileName = file.toLowerCase();
      if (fileName.includes('route')) layers.add('presentation');
      if (fileName.includes('controller')) layers.add('presentation');
      if (fileName.includes('service')) layers.add('business');
      if (fileName.includes('model')) layers.add('data');
      if (fileName.includes('repository')) layers.add('data');
      if (fileName.includes('middleware')) layers.add('infrastructure');
    }
    
    return layers;
  }

  private buildAuthImplementation(evidence: CrossFileEvidence[]): any {
    // Convert cross-file evidence to implementation pattern format
    const authEvidence = evidence.filter(e => 
      e.type === 'middleware_usage' || 
      e.snippet.toLowerCase().includes('auth')
    );

    return {
      userProperty: this.extractUserProperty(authEvidence),
      tokenLocation: this.extractTokenLocation(authEvidence),
      errorResponse: {
        format: '{message: string}',
        statusCode: 401
      },
      middlewarePattern: 'app.use(auth)',
      confidence: Math.min(100, authEvidence.length * 20),
      evidence: authEvidence.map(e => `${e.file}: ${e.snippet}`)
    };
  }

  private buildAPIImplementation(evidence: CrossFileEvidence[]): any {
    const apiEvidence = evidence.filter(e => 
      e.type === 'function_call' || 
      e.snippet.toLowerCase().includes('response')
    );

    return {
      successFormat: 'bare object',
      errorFormat: '{message: string}',
      statusCodeUsage: 'explicit codes',
      wrapperPattern: 'conditional',
      confidence: Math.min(100, apiEvidence.length * 15),
      evidence: apiEvidence.map(e => `${e.file}: ${e.snippet}`)
    };
  }

  private buildErrorImplementation(evidence: CrossFileEvidence[]): any {
    const errorEvidence = evidence.filter(e => 
      e.type === 'error_handling' || 
      e.snippet.toLowerCase().includes('error')
    );

    return {
      catchPattern: 'global middleware',
      errorStructure: '{message: string}',
      loggingIntegration: 'integrated',
      propagationStyle: 'throw exceptions',
      confidence: Math.min(100, errorEvidence.length * 25),
      evidence: errorEvidence.map(e => `${e.file}: ${e.snippet}`)
    };
  }

  private extractUserProperty(evidence: CrossFileEvidence[]): string {
    // Simple heuristic - look for common patterns in evidence
    for (const e of evidence) {
      if (e.snippet.includes('req.user')) return 'req.user';
      if (e.snippet.includes('req.auth')) return 'req.auth';
      if (e.snippet.includes('req.context.user')) return 'req.context.user';
    }
    return 'Unknown';
  }

  private extractTokenLocation(evidence: CrossFileEvidence[]): string {
    for (const e of evidence) {
      if (e.snippet.toLowerCase().includes('cookie')) return 'httpOnly cookie';
      if (e.snippet.toLowerCase().includes('header') || e.snippet.toLowerCase().includes('bearer')) return 'authorization header';
    }
    return 'Unknown';
  }
}