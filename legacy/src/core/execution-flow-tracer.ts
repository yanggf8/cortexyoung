/**
 * Execution Flow Tracer
 * Traces execution paths across files to understand cross-file architectural patterns
 */

import type { TSESTree } from '@typescript-eslint/typescript-estree';
import type {
  DependencyGraph,
  ExecutionPath,
  ExecutionStep,
  CrossFilePatternType,
  SymbolUsage
} from '../types/cross-file-analysis.js';
import { SymbolResolver } from './symbol-resolver.js';

export interface FlowTraceResult {
  /** All discovered execution paths */
  executionPaths: ExecutionPath[];
  /** Entry points (routes, main functions, etc.) */
  entryPoints: string[];
  /** Flow statistics */
  stats: FlowStats;
}

export interface FlowStats {
  /** Total execution paths found */
  totalPaths: number;
  /** Average path length */
  averagePathLength: number;
  /** Most common patterns */
  patternFrequency: Map<CrossFilePatternType, number>;
  /** Files most frequently involved in flows */
  highTrafficFiles: string[];
}

export interface CallSite {
  /** File containing the call */
  file: string;
  /** Function making the call */
  caller: string;
  /** Function being called */
  callee: string;
  /** Target file (if cross-file) */
  targetFile?: string;
  /** Line number */
  lineNumber: number;
  /** AST node */
  astNode?: TSESTree.Node;
}

export class ExecutionFlowTracer {
  private graph: DependencyGraph;
  private symbolResolver: SymbolResolver;
  private callSites: Map<string, CallSite[]> = new Map();
  private entryPoints: Set<string> = new Set();

  constructor(graph: DependencyGraph, symbolResolver: SymbolResolver) {
    this.graph = graph;
    this.symbolResolver = symbolResolver;
  }

  /**
   * Trace all execution flows in the project
   */
  traceExecutionFlows(): FlowTraceResult {
    console.log('🔄 Tracing execution flows...');

    // Step 1: Find all call sites
    this.findAllCallSites();
    console.log(`📞 Found ${this.getTotalCallSites()} call sites`);

    // Step 2: Identify entry points
    this.identifyEntryPoints();
    console.log(`🚪 Identified ${this.entryPoints.size} entry points`);

    // Step 3: Trace execution paths from entry points
    const executionPaths = this.traceFromEntryPoints();
    console.log(`🛤️ Traced ${executionPaths.length} execution paths`);

    // Step 4: Classify patterns
    this.classifyExecutionPatterns(executionPaths);
    console.log('📊 Classified execution patterns');

    const stats = this.calculateFlowStats(executionPaths);

    return {
      executionPaths,
      entryPoints: Array.from(this.entryPoints),
      stats
    };
  }

  /**
   * Trace execution flow from a specific entry point
   */
  traceFromEntry(entryFile: string, entryFunction: string): ExecutionPath[] {
    const paths: ExecutionPath[] = [];
    const visited = new Set<string>();
    const maxDepth = 15; // Prevent infinite recursion

    const trace = (
      currentFile: string, 
      currentFunction: string, 
      currentPath: ExecutionStep[], 
      depth: number
    ) => {
      if (depth > maxDepth) return;

      const stepKey = `${currentFile}:${currentFunction}`;
      if (visited.has(stepKey)) return;

      visited.add(stepKey);

      const currentStep: ExecutionStep = {
        file: currentFile,
        function: currentFunction,
        lineNumber: this.findFunctionLineNumber(currentFile, currentFunction),
        callsTo: []
      };

      // Find all calls made from this function
      const calls = this.findCallsFromFunction(currentFile, currentFunction);
      
      for (const call of calls) {
        const nextStep = this.resolveCall(call);
        if (nextStep) {
          currentStep.callsTo!.push(nextStep);
          
          // Recursively trace further
          if (nextStep.file !== currentFile) {
            // Cross-file call
            trace(nextStep.file, nextStep.function, [...currentPath, currentStep], depth + 1);
          } else {
            // Same file call
            trace(currentFile, nextStep.function, [...currentPath, currentStep], depth + 1);
          }
        }
      }

      // If this is a leaf node or reached significant depth, create a path
      if (currentStep.callsTo!.length === 0 || depth >= 5) {
        const path: ExecutionPath = {
          entry: `${entryFile}:${entryFunction}`,
          callChain: [...currentPath, currentStep],
          filesInvolved: this.extractFilesFromChain([...currentPath, currentStep]),
          patternType: this.classifyPatternType([...currentPath, currentStep])
        };
        paths.push(path);
      }

      visited.delete(stepKey);
    };

    trace(entryFile, entryFunction, [], 0);
    return paths;
  }

  /**
   * Find all authentication flows
   */
  traceAuthenticationFlows(): ExecutionPath[] {
    const authPaths: ExecutionPath[] = [];
    
    // Look for authentication middleware and routes
    for (const [filePath, fileNode] of this.graph.files) {
      if (this.isAuthenticationFile(filePath, fileNode.content)) {
        const authFunctions = this.findAuthenticationFunctions(filePath, fileNode.content);
        
        for (const authFunc of authFunctions) {
          const paths = this.traceFromEntry(filePath, authFunc);
          authPaths.push(...paths.filter(p => p.patternType === 'auth_flow'));
        }
      }
    }

    return authPaths;
  }

  /**
   * Find all API response flows
   */
  traceAPIResponseFlows(): ExecutionPath[] {
    const apiPaths: ExecutionPath[] = [];
    
    // Look for route handlers and controllers
    for (const [filePath, fileNode] of this.graph.files) {
      if (this.isAPIFile(filePath, fileNode.content)) {
        const apiFunctions = this.findAPIFunctions(filePath, fileNode.content);
        
        for (const apiFunc of apiFunctions) {
          const paths = this.traceFromEntry(filePath, apiFunc);
          apiPaths.push(...paths.filter(p => p.patternType === 'api_response_flow'));
        }
      }
    }

    return apiPaths;
  }

  /**
   * Find MVC execution flows
   */
  traceMVCFlows(): ExecutionPath[] {
    const mvcPaths: ExecutionPath[] = [];
    
    // Find route -> controller -> service -> model flows
    for (const [filePath, fileNode] of this.graph.files) {
      if (fileNode.fileType === 'route') {
        const routeFunctions = this.findRouteFunctions(filePath, fileNode.content);
        
        for (const routeFunc of routeFunctions) {
          const paths = this.traceFromEntry(filePath, routeFunc);
          mvcPaths.push(...paths.filter(p => p.patternType === 'mvc_flow'));
        }
      }
    }

    return mvcPaths;
  }

  private findAllCallSites(): void {
    for (const [filePath, fileNode] of this.graph.files) {
      if (!fileNode.ast) continue;

      const fileCalls: CallSite[] = [];
      
      this.walkAST(fileNode.ast, (node: TSESTree.Node) => {
        if (node.type === 'CallExpression') {
          const callSite = this.parseCallExpression(node, filePath);
          if (callSite) {
            fileCalls.push(callSite);
          }
        }
      });

      if (fileCalls.length > 0) {
        this.callSites.set(filePath, fileCalls);
      }
    }
  }

  private parseCallExpression(node: TSESTree.CallExpression, filePath: string): CallSite | null {
    let callee: string;
    let targetFile: string | undefined;

    switch (node.callee.type) {
      case 'Identifier':
        callee = node.callee.name;
        break;
      case 'MemberExpression':
        if (node.callee.property.type === 'Identifier') {
          callee = node.callee.property.name;
          
          // Check if this is a cross-file call
          if (node.callee.object.type === 'Identifier') {
            const objectName = node.callee.object.name;
            targetFile = this.resolveImportedSymbolFile(objectName, filePath);
          }
        } else {
          return null;
        }
        break;
      default:
        return null;
    }

    return {
      file: filePath,
      caller: this.findContainingFunction(node, filePath) || 'anonymous',
      callee,
      targetFile,
      lineNumber: node.loc?.start.line || 0,
      astNode: node
    };
  }

  private identifyEntryPoints(): void {
    for (const [filePath, fileNode] of this.graph.files) {
      // Route handlers are entry points
      if (fileNode.fileType === 'route' || this.isRouteFile(filePath, fileNode.content)) {
        this.entryPoints.add(filePath);
      }

      // Main application files are entry points
      if (fileNode.fileType === 'main') {
        this.entryPoints.add(filePath);
      }

      // Files with HTTP server setup
      if (this.hasServerSetup(fileNode.content)) {
        this.entryPoints.add(filePath);
      }

      // Middleware files can be entry points
      if (fileNode.fileType === 'middleware') {
        this.entryPoints.add(filePath);
      }
    }
  }

  private traceFromEntryPoints(): ExecutionPath[] {
    const allPaths: ExecutionPath[] = [];

    for (const entryFile of this.entryPoints) {
      const entryFunctions = this.findEntryFunctions(entryFile);
      
      for (const entryFunc of entryFunctions) {
        const paths = this.traceFromEntry(entryFile, entryFunc);
        allPaths.push(...paths);
      }
    }

    return allPaths;
  }

  private classifyExecutionPatterns(paths: ExecutionPath[]): void {
    for (const path of paths) {
      path.patternType = this.classifyPatternType(path.callChain);
    }
  }

  private classifyPatternType(chain: ExecutionStep[]): CrossFilePatternType {
    const filesInChain = chain.map(step => step.file);
    const uniqueFiles = [...new Set(filesInChain)];
    
    // Analyze file types in the chain
    const fileTypes = uniqueFiles.map(file => {
      const fileNode = this.graph.files.get(file);
      return fileNode ? fileNode.fileType : 'unknown';
    });

    // MVC Pattern: route -> controller -> service/model
    if (fileTypes.includes('route') && fileTypes.includes('controller')) {
      if (fileTypes.includes('service') || fileTypes.includes('model')) {
        return 'mvc_flow';
      }
    }

    // Authentication flow: middleware -> route
    if (fileTypes.includes('middleware') && 
        chain.some(step => this.isAuthFunction(step.function))) {
      return 'auth_flow';
    }

    // Service composition: service -> service
    const serviceCount = fileTypes.filter(type => type === 'service').length;
    if (serviceCount >= 2) {
      return 'service_composition';
    }

    // API response flow: route/controller -> response
    if (fileTypes.includes('route') || fileTypes.includes('controller')) {
      if (chain.some(step => this.isResponseFunction(step.function))) {
        return 'api_response_flow';
      }
    }

    // Middleware chain
    const middlewareCount = fileTypes.filter(type => type === 'middleware').length;
    if (middlewareCount >= 2) {
      return 'middleware_chain';
    }

    // Layered architecture
    if (uniqueFiles.length >= 3 && this.hasLayeredStructure(fileTypes)) {
      return 'layered_architecture';
    }

    // Default to mixed if cross-file
    return uniqueFiles.length > 1 ? 'mixed' : 'layered_architecture';
  }

  private findCallsFromFunction(filePath: string, functionName: string): CallSite[] {
    const fileCalls = this.callSites.get(filePath) || [];
    return fileCalls.filter(call => call.caller === functionName);
  }

  private resolveCall(call: CallSite): ExecutionStep | null {
    if (call.targetFile) {
      // Cross-file call
      return {
        file: call.targetFile,
        function: call.callee,
        lineNumber: this.findFunctionLineNumber(call.targetFile, call.callee)
      };
    } else {
      // Same-file call
      return {
        file: call.file,
        function: call.callee,
        lineNumber: this.findFunctionLineNumber(call.file, call.callee)
      };
    }
  }

  private extractFilesFromChain(chain: ExecutionStep[]): string[] {
    const files = chain.map(step => step.file);
    return [...new Set(files)];
  }

  private findFunctionLineNumber(filePath: string, functionName: string): number {
    const fileNode = this.graph.files.get(filePath);
    if (!fileNode?.ast) return 0;

    let lineNumber = 0;
    this.walkAST(fileNode.ast, (node: TSESTree.Node) => {
      if ((node.type === 'FunctionDeclaration' || node.type === 'FunctionExpression') &&
          (node as any).id?.name === functionName) {
        lineNumber = node.loc?.start.line || 0;
      }
    });

    return lineNumber;
  }

  private findContainingFunction(node: TSESTree.Node, filePath: string): string | null {
    const fileNode = this.graph.files.get(filePath);
    if (!fileNode?.ast) return null;

    // This is a simplified implementation
    // In a real implementation, you'd need to walk up the AST to find the containing function
    return 'unknown';
  }

  private resolveImportedSymbolFile(symbolName: string, filePath: string): string | undefined {
    const resolvedSymbol = this.symbolResolver.findSymbolDefinition(symbolName, filePath);
    return resolvedSymbol?.definedIn;
  }

  private isAuthenticationFile(filePath: string, content: string): boolean {
    const fileName = filePath.toLowerCase();
    return fileName.includes('auth') || 
           fileName.includes('middleware') ||
           content.includes('passport') ||
           content.includes('jwt') ||
           content.includes('authenticate');
  }

  private isAPIFile(filePath: string, content: string): boolean {
    const fileName = filePath.toLowerCase();
    return fileName.includes('route') ||
           fileName.includes('controller') ||
           fileName.includes('api') ||
           content.includes('res.json') ||
           content.includes('app.get') ||
           content.includes('router.');
  }

  private isRouteFile(filePath: string, content: string): boolean {
    return content.includes('app.get') ||
           content.includes('app.post') ||
           content.includes('router.get') ||
           content.includes('router.post') ||
           filePath.toLowerCase().includes('route');
  }

  private hasServerSetup(content: string): boolean {
    return content.includes('app.listen') ||
           content.includes('server.listen') ||
           content.includes('createServer');
  }

  private findEntryFunctions(filePath: string): string[] {
    const fileNode = this.graph.files.get(filePath);
    if (!fileNode?.ast) return [];

    const functions: string[] = [];
    
    this.walkAST(fileNode.ast, (node: TSESTree.Node) => {
      if (node.type === 'FunctionDeclaration' && (node as any).id?.name) {
        functions.push((node as any).id.name);
      }
      // Also look for route handlers
      if (node.type === 'CallExpression' && this.isRouteCall(node)) {
        functions.push('routeHandler');
      }
    });

    return functions.length > 0 ? functions : ['main'];
  }

  private findAuthenticationFunctions(filePath: string, content: string): string[] {
    // Simplified - look for common auth function names
    const authFunctions: string[] = [];
    const lines = content.split('\n');
    
    for (const line of lines) {
      if (line.includes('function') && 
          (line.includes('auth') || line.includes('verify') || line.includes('check'))) {
        // Extract function name (simplified)
        const match = line.match(/function\s+(\w+)/);
        if (match) {
          authFunctions.push(match[1]);
        }
      }
    }

    return authFunctions.length > 0 ? authFunctions : ['authenticate'];
  }

  private findAPIFunctions(filePath: string, content: string): string[] {
    const apiFunctions: string[] = [];
    const lines = content.split('\n');
    
    for (const line of lines) {
      if (line.includes('function') || line.includes('=>')) {
        if (line.includes('req') && line.includes('res')) {
          // Extract function name (simplified)
          const match = line.match(/function\s+(\w+)/) || line.match(/const\s+(\w+)\s*=/);
          if (match) {
            apiFunctions.push(match[1]);
          }
        }
      }
    }

    return apiFunctions.length > 0 ? apiFunctions : ['handler'];
  }

  private findRouteFunctions(filePath: string, content: string): string[] {
    // Find route handler functions
    return this.findAPIFunctions(filePath, content);
  }

  private isAuthFunction(functionName: string): boolean {
    const name = functionName.toLowerCase();
    return name.includes('auth') || 
           name.includes('verify') || 
           name.includes('check') ||
           name.includes('validate');
  }

  private isResponseFunction(functionName: string): boolean {
    const name = functionName.toLowerCase();
    return name.includes('response') ||
           name.includes('send') ||
           name.includes('json') ||
           name.includes('reply');
  }

  private hasLayeredStructure(fileTypes: string[]): boolean {
    // Check if file types suggest a layered architecture
    const layers = ['route', 'controller', 'service', 'model'];
    let layerCount = 0;
    
    for (const layer of layers) {
      if (fileTypes.includes(layer)) {
        layerCount++;
      }
    }
    
    return layerCount >= 3;
  }

  private isRouteCall(node: TSESTree.CallExpression): boolean {
    if (node.callee.type === 'MemberExpression') {
      const property = node.callee.property;
      if (property.type === 'Identifier') {
        const method = property.name;
        return ['get', 'post', 'put', 'delete', 'patch'].includes(method);
      }
    }
    return false;
  }

  private getTotalCallSites(): number {
    return Array.from(this.callSites.values())
      .reduce((total, calls) => total + calls.length, 0);
  }

  private calculateFlowStats(paths: ExecutionPath[]): FlowStats {
    const patternFrequency = new Map<CrossFilePatternType, number>();
    const fileFrequency = new Map<string, number>();
    let totalSteps = 0;

    for (const path of paths) {
      // Pattern frequency
      const currentCount = patternFrequency.get(path.patternType) || 0;
      patternFrequency.set(path.patternType, currentCount + 1);

      // File frequency
      for (const file of path.filesInvolved) {
        const currentFileCount = fileFrequency.get(file) || 0;
        fileFrequency.set(file, currentFileCount + 1);
      }

      totalSteps += path.callChain.length;
    }

    const averagePathLength = paths.length > 0 ? totalSteps / paths.length : 0;
    
    // Get top 5 high traffic files
    const highTrafficFiles = Array.from(fileFrequency.entries())
      .sort((a, b) => b[1] - a[1])
      .slice(0, 5)
      .map(entry => entry[0]);

    return {
      totalPaths: paths.length,
      averagePathLength: Math.round(averagePathLength * 100) / 100,
      patternFrequency,
      highTrafficFiles
    };
  }

  private walkAST(node: TSESTree.Node | TSESTree.Program, callback: (node: TSESTree.Node) => void) {
    callback(node);

    for (const key in node) {
      if (key === 'parent' || key === 'range' || key === 'loc') continue;
      
      const child = (node as any)[key];
      if (Array.isArray(child)) {
        for (const item of child) {
          if (item && typeof item === 'object' && item.type) {
            this.walkAST(item, callback);
          }
        }
      } else if (child && typeof child === 'object' && child.type) {
        this.walkAST(child, callback);
      }
    }
  }
}