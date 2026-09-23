import { glob } from 'glob';
import { readFileSync, existsSync, statSync } from 'fs';
import { join, dirname } from 'path';
import type { 
  ProjectContextInfo, 
  ImplementationPatterns, 
  PatternDetectionSignal,
  ConfidenceCalculation 
} from '../types/patterns.js';

export class EnhancedProjectDetector {
  private projectRoot: string;

  constructor(projectRoot: string = process.cwd()) {
    this.projectRoot = projectRoot;
  }

  /**
   * Main detection method - analyzes project and returns comprehensive context
   */
  detectCurrentContext(): ProjectContextInfo {
    const basicContext = this.detectBasicProjectInfo();
    const implementationPatterns = this.detectImplementationPatterns();
    
    return {
      ...basicContext,
      implementationPatterns,
      lastUpdated: new Date().toISOString()
    };
  }

  /**
   * Detect basic project information from package.json and file structure
   */
  private detectBasicProjectInfo() {
    const pkg = this.readPackageJson();
    const allFiles = this.findAllJSTSFiles();
    const mainFiles = this.detectMainFiles(allFiles);
    
    const projectType = this.determineProjectType(pkg, mainFiles);
    const language = this.determineLanguage(pkg);
    const framework = this.determineFramework(pkg, mainFiles);
    const packageManager = this.determinePackageManager();
    
    return {
      projectType,
      language,
      framework,
      packageManager,
      directories: this.mapKeyDirectories(),
      dependencies: Object.keys(pkg.dependencies || {}),
      scripts: pkg.scripts || {},
      mainFiles: mainFiles.map(f => f.replace(this.projectRoot, ''))
    };
  }

  /**
   * Core implementation pattern detection
   */
  private detectImplementationPatterns(): ImplementationPatterns {
    return {
      authentication: this.detectAuthImplementation(),
      apiResponses: this.detectAPIResponseImplementation(),
      errorHandling: this.detectErrorHandlingImplementation()
    };
  }

  /**
   * Detect authentication implementation patterns
   */
  private detectAuthImplementation() {
    // Broader file scanning - search all JS/TS files for auth patterns
    const allFiles = this.findAllJSTSFiles();
    const authFiles = this.filterFilesByAuthPatterns(allFiles);
    const evidence: string[] = [];
    const signals: PatternDetectionSignal[] = [];
    
    let userProperty = 'Unknown';
    let tokenLocation = 'Unknown';
    let errorFormat = 'Unknown';
    let errorStatusCode = 0;
    let middlewarePattern = 'Unknown';

    if (authFiles.length > 0) {
      for (const file of authFiles.slice(0, 3)) { // Analyze first 3 auth files
        const content = this.readFileContent(file);
        const relativeFile = file.replace(this.projectRoot, '');

        // Detect user property pattern with flexible matching
        const userPatterns = ['req.user', 'req.context.user', 'req.auth', 'request.user', 'ctx.user', 'context.user'];
        const userDetection = this.detectFlexiblePattern(content, userPatterns);
        
        if (userDetection) {
          userProperty = userDetection.pattern;
          const confidenceWeight = Math.round(30 * userDetection.confidence);
          evidence.push(`${relativeFile}: ${userDetection.pattern} usage (${Math.round(userDetection.confidence * 100)}% confidence)`);
          signals.push({ weight: confidenceWeight, detected: true, evidence: `${relativeFile}: ${userDetection.pattern}` });
        }

        // Detect token location
        if (content.includes('httpOnly') || content.includes('cookie')) {
          tokenLocation = 'httpOnly cookie';
          evidence.push(`${relativeFile}: cookie-based auth`);
          signals.push({ weight: 25, detected: true });
        } else if (content.includes('authorization') || content.includes('Bearer')) {
          tokenLocation = 'authorization header';
          evidence.push(`${relativeFile}: header-based auth`);
          signals.push({ weight: 25, detected: true });
        }

        // Detect error response format with flexible patterns
        const errorPatterns = ['{error:', '{ error:', '{message:', '{ message:', 'error:', 'message:'];
        const errorDetection = this.detectFlexiblePattern(content, errorPatterns);
        
        if (errorDetection) {
          if (errorDetection.pattern.includes('error')) {
            errorFormat = '{error: string}';
          } else if (errorDetection.pattern.includes('message')) {
            errorFormat = '{message: string}';
          }
          const confidenceWeight = Math.round(20 * errorDetection.confidence);
          signals.push({ weight: confidenceWeight, detected: true });
        }

        // Detect status codes
        if (content.includes('401')) {
          errorStatusCode = 401;
          signals.push({ weight: 15, detected: true });
        } else if (content.includes('403')) {
          errorStatusCode = 403;
          signals.push({ weight: 15, detected: true });
        }

        // Detect middleware pattern
        if (content.includes('app.use')) {
          middlewarePattern = 'app.use(auth)';
          signals.push({ weight: 20, detected: true });
        } else if (content.includes('@authenticated') || content.includes('@auth')) {
          middlewarePattern = '@authenticated';
          signals.push({ weight: 20, detected: true });
        }
      }
    } else {
      signals.push({ weight: 30, detected: false }); // No auth files found
    }

    const confidence = this.calculateConfidence(signals);

    return {
      userProperty,
      tokenLocation,
      errorResponse: {
        format: errorFormat,
        statusCode: errorStatusCode
      },
      middlewarePattern,
      confidence: confidence.totalConfidence,
      evidence
    };
  }

  /**
   * Detect API response implementation patterns
   */
  private detectAPIResponseImplementation() {
    // Broader file scanning - search all JS/TS files for API patterns
    const allFiles = this.findAllJSTSFiles();
    const apiFiles = this.filterFilesByAPIPatterns(allFiles);
    const evidence: string[] = [];
    const signals: PatternDetectionSignal[] = [];
    
    let successFormat = 'Unknown';
    let errorFormat = 'Unknown';
    let statusCodeUsage = 'Unknown';
    let wrapperPattern = 'Unknown';

    if (apiFiles.length > 0) {
      let dataWrapperCount = 0;
      let resultWrapperCount = 0;
      let bareObjectCount = 0;

      // Analyze multiple API files for consistent patterns
      for (const file of apiFiles.slice(0, 3)) {
        const content = this.readFileContent(file);
        const relativeFile = file.replace(this.projectRoot, '');

        // Detect success response format with flexible patterns
        const dataPatterns = ['{data:', '{ data:', 'data:'];
        const resultPatterns = ['{result:', '{ result:', 'result:'];
        const responsePatterns = ['res.json(', 'response.json(', 'return '];
        
        const dataDetection = this.detectFlexiblePattern(content, dataPatterns);
        const resultDetection = this.detectFlexiblePattern(content, resultPatterns);
        const responseDetection = this.detectFlexiblePattern(content, responsePatterns);
        
        if (dataDetection) {
          dataWrapperCount++;
          evidence.push(`${relativeFile}: {data: any} format (${Math.round(dataDetection.confidence * 100)}% confidence)`);
        } else if (resultDetection) {
          resultWrapperCount++;
          evidence.push(`${relativeFile}: {result: any} format (${Math.round(resultDetection.confidence * 100)}% confidence)`);
        } else if (responseDetection) {
          bareObjectCount++;
          evidence.push(`${relativeFile}: bare object response (${Math.round(responseDetection.confidence * 100)}% confidence)`);
        }

        // Detect error response format with flexible patterns
        const errorResponsePatterns = ['{error:', '{ error:', '{message:', '{ message:', 'error:', 'message:'];
        const errorResponseDetection = this.detectFlexiblePattern(content, errorResponsePatterns);
        
        if (errorResponseDetection) {
          if (errorResponseDetection.pattern.includes('error')) {
            errorFormat = '{error: string}';
          } else if (errorResponseDetection.pattern.includes('message')) {
            errorFormat = '{message: string}';
          }
          evidence.push(`${relativeFile}: ${errorFormat} format (${Math.round(errorResponseDetection.confidence * 100)}% confidence)`);
          const confidenceWeight = Math.round(15 * errorResponseDetection.confidence);
          signals.push({ weight: confidenceWeight, detected: true });
        }

        // Detect status code usage
        if (content.includes('.status(') || content.includes('statusCode')) {
          statusCodeUsage = 'explicit codes';
          signals.push({ weight: 10, detected: true });
        }
      }

      // Determine dominant success format
      if (dataWrapperCount > resultWrapperCount && dataWrapperCount > bareObjectCount) {
        successFormat = '{data: any}';
        wrapperPattern = 'always wrapped';
        signals.push({ weight: 30, detected: true });
      } else if (resultWrapperCount > dataWrapperCount && resultWrapperCount > bareObjectCount) {
        successFormat = '{result: any}';
        wrapperPattern = 'always wrapped';
        signals.push({ weight: 30, detected: true });
      } else if (bareObjectCount > 0) {
        successFormat = 'bare object';
        wrapperPattern = 'conditional';
        signals.push({ weight: 25, detected: true });
      }

      if (statusCodeUsage === 'Unknown') {
        statusCodeUsage = 'default 200/500';
        signals.push({ weight: 5, detected: true });
      }
    } else {
      signals.push({ weight: 30, detected: false }); // No API files found
    }

    const confidence = this.calculateConfidence(signals);

    return {
      successFormat,
      errorFormat,
      statusCodeUsage,
      wrapperPattern,
      confidence: confidence.totalConfidence,
      evidence
    };
  }

  /**
   * Detect error handling implementation patterns
   */
  private detectErrorHandlingImplementation() {
    // Broader file scanning - search all JS/TS files for error patterns
    const allFiles = this.findAllJSTSFiles();
    const errorFiles = this.filterFilesByErrorPatterns(allFiles);
    const mainFiles = this.detectMainFiles(allFiles);
    const evidence: string[] = [];
    const signals: PatternDetectionSignal[] = [];
    
    let catchPattern = 'Unknown';
    let errorStructure = 'Unknown';
    let loggingIntegration = 'Unknown';
    let propagationStyle = 'Unknown';

    // Check for global error middleware
    if (errorFiles.length > 0) {
      const errorContent = this.readFileContent(errorFiles[0]);
      const relativeFile = errorFiles[0].replace(this.projectRoot, '');
      
      catchPattern = 'global middleware';
      evidence.push(`${relativeFile}: global error handler`);
      signals.push({ weight: 40, detected: true });

      // Detect error structure from middleware
      if (errorContent.includes('{error:') || errorContent.includes('{ error:')) {
        errorStructure = '{error: string, code?: string}';
        signals.push({ weight: 25, detected: true });
      } else if (errorContent.includes('{message:') || errorContent.includes('{ message:')) {
        errorStructure = '{message: string}';
        signals.push({ weight: 25, detected: true });
      }

      // Check for logging integration
      if (errorContent.includes('console.log') || errorContent.includes('logger') || errorContent.includes('winston')) {
        loggingIntegration = 'integrated';
        signals.push({ weight: 15, detected: true });
      }
    }

    // Check main files and other files for try/catch patterns
    const filesToCheck = catchPattern === 'Unknown' ? [...mainFiles, ...errorFiles.filter(f => !mainFiles.includes(f))] : [];
    
    if (filesToCheck.length > 0 && catchPattern === 'Unknown') {
      const fileContent = this.readFileContent(filesToCheck[0]);
      const relativeFile = filesToCheck[0].replace(this.projectRoot, '');
      
      if (fileContent.includes('try {') && fileContent.includes('catch')) {
        catchPattern = 'try/catch blocks';
        evidence.push(`${relativeFile}: try/catch usage`);
        signals.push({ weight: 30, detected: true });
      }

      // Check for Result type patterns
      if (fileContent.includes('Result<') || fileContent.includes('Either<')) {
        catchPattern = 'Result types';
        propagationStyle = 'return errors';
        evidence.push(`${relativeFile}: Result type usage`);
        signals.push({ weight: 35, detected: true });
      } else if (fileContent.includes('throw')) {
        propagationStyle = 'throw exceptions';
        signals.push({ weight: 15, detected: true });
      }
    }

    if (loggingIntegration === 'Unknown') {
      loggingIntegration = 'separate';
      signals.push({ weight: 5, detected: true });
    }

    const confidence = this.calculateConfidence(signals);

    return {
      catchPattern,
      errorStructure,
      loggingIntegration,
      propagationStyle,
      confidence: confidence.totalConfidence,
      evidence
    };
  }

  /**
   * Enhanced pattern matching with context validation and flexible detection
   */
  private validatePatternInContext(content: string, pattern: string, contextKeywords: string[]): boolean {
    const lines = content.split('\n');
    for (let i = 0; i < lines.length; i++) {
      if (lines[i].includes(pattern)) {
        // Check surrounding context for validation
        const contextLines = lines.slice(Math.max(0, i - 2), Math.min(lines.length, i + 3));
        const contextText = contextLines.join(' ').toLowerCase();
        return contextKeywords.some(keyword => contextText.includes(keyword.toLowerCase()));
      }
    }
    return false;
  }

  /**
   * Flexible pattern detection that works across different code styles
   */
  private detectFlexiblePattern(content: string, patterns: string[]): { pattern: string; confidence: number } | null {
    const normalizedContent = content.replace(/\s+/g, ' ').toLowerCase();
    
    for (const pattern of patterns) {
      // Direct match
      if (normalizedContent.includes(pattern.toLowerCase())) {
        return { pattern, confidence: 1.0 };
      }
      
      // Partial match with variations
      const patternParts = pattern.split('.');
      if (patternParts.length > 1) {
        const lastPart = patternParts[patternParts.length - 1];
        if (normalizedContent.includes(lastPart.toLowerCase())) {
          return { pattern, confidence: 0.7 };
        }
      }
      
      // Similar pattern detection (e.g., request.user vs req.user)
      const variations = this.generatePatternVariations(pattern);
      for (const variation of variations) {
        if (normalizedContent.includes(variation.toLowerCase())) {
          return { pattern, confidence: 0.8 };
        }
      }
    }
    
    return null;
  }

  /**
   * Generate common variations of a pattern
   */
  private generatePatternVariations(pattern: string): string[] {
    const variations: string[] = [];
    
    // Common abbreviations and expansions
    const replacements = [
      { from: 'req', to: 'request' },
      { from: 'request', to: 'req' },
      { from: 'res', to: 'response' },
      { from: 'response', to: 'res' },
      { from: 'auth', to: 'authentication' },
      { from: 'authentication', to: 'auth' },
      { from: 'ctx', to: 'context' },
      { from: 'context', to: 'ctx' }
    ];
    
    for (const replacement of replacements) {
      if (pattern.includes(replacement.from)) {
        variations.push(pattern.replace(replacement.from, replacement.to));
      }
    }
    
    return variations;
  }

  /**
   * Multi-signal confidence calculation
   */
  private calculateConfidence(signals: PatternDetectionSignal[]): ConfidenceCalculation {
    const totalWeight = signals.reduce((sum, signal) => sum + signal.weight, 0);
    const detectedWeight = signals.reduce((sum, signal) => 
      sum + (signal.detected ? signal.weight : 0), 0);
    
    const totalConfidence = totalWeight > 0 ? Math.round((detectedWeight / totalWeight) * 100) : 0;
    
    let uncertaintyWarning: string | undefined;
    if (totalConfidence < 60) {
      uncertaintyWarning = `⚠️ Pattern unclear (${totalConfidence}% confidence) - Ask before making assumptions`;
    } else if (totalConfidence < 80) {
      uncertaintyWarning = `⚠️ Pattern detected but uncertain (${totalConfidence}% confidence) - Verify before critical decisions`;
    }

    return {
      signals,
      totalConfidence: Math.min(totalConfidence, 100),
      uncertaintyWarning
    };
  }

  /**
   * Find all JavaScript and TypeScript files in the project
   */
  private findAllJSTSFiles(): string[] {
    try {
      // Scan entire project for all JS/TS files, excluding node_modules and common build dirs
      const allFiles = glob.sync('**/*.{js,ts,jsx,tsx}', {
        cwd: this.projectRoot,
        absolute: true,
        ignore: [
          'node_modules/**',
          'dist/**',
          'build/**',
          '.next/**',
          'coverage/**',
          '*.d.ts' // Exclude type definition files
        ]
      });
      return allFiles;
    } catch (error) {
      return [];
    }
  }

  /**
   * Filter files that likely contain authentication patterns
   */
  private filterFilesByAuthPatterns(files: string[]): string[] {
    return files.filter(file => {
      const fileName = file.toLowerCase();
      const content = this.readFileContent(file);
      
      // File name suggests auth functionality
      const hasAuthFileName = fileName.includes('auth') || 
        fileName.includes('middleware') || 
        fileName.includes('guard') ||
        fileName.includes('jwt') ||
        fileName.includes('passport') ||
        fileName.includes('session');
      
      // Content suggests auth functionality
      const hasAuthContent = content.includes('req.user') ||
        content.includes('req.auth') ||
        content.includes('req.context.user') ||
        content.includes('authorization') ||
        content.includes('Bearer') ||
        content.includes('jwt') ||
        content.includes('passport') ||
        content.includes('authenticate');
      
      return hasAuthFileName || hasAuthContent;
    });
  }

  /**
   * Filter files that likely contain API response patterns
   */
  private filterFilesByAPIPatterns(files: string[]): string[] {
    return files.filter(file => {
      const fileName = file.toLowerCase();
      const content = this.readFileContent(file);
      
      // File name suggests API functionality
      const hasAPIFileName = fileName.includes('controller') ||
        fileName.includes('route') ||
        fileName.includes('handler') ||
        fileName.includes('api') ||
        fileName.includes('endpoint');
      
      // Content suggests API functionality
      const hasAPIContent = content.includes('res.json') ||
        content.includes('res.send') ||
        content.includes('response') ||
        content.includes('req.') ||
        content.includes('app.get') ||
        content.includes('app.post') ||
        content.includes('router.') ||
        content.includes('express') ||
        content.includes('fastify') ||
        content.includes('return ') && (content.includes('{data:') || content.includes('{result:'));
      
      return hasAPIFileName || hasAPIContent;
    });
  }

  /**
   * Filter files that likely contain error handling patterns
   */
  private filterFilesByErrorPatterns(files: string[]): string[] {
    return files.filter(file => {
      const fileName = file.toLowerCase();
      const content = this.readFileContent(file);
      
      // File name suggests error handling
      const hasErrorFileName = fileName.includes('error') ||
        fileName.includes('exception') ||
        fileName.includes('catch');
      
      // Content suggests error handling
      const hasErrorContent = content.includes('try {') ||
        content.includes('catch') ||
        content.includes('throw') ||
        content.includes('Error') ||
        content.includes('app.use((err') ||
        content.includes('(error, req, res') ||
        content.includes('Result<') ||
        content.includes('Either<');
      
      return hasErrorFileName || hasErrorContent;
    });
  }

  /**
   * Detect main application files (server.js, app.js, index.js, main.ts, etc.)
   */
  private detectMainFiles(files: string[]): string[] {
    const mainFileNames = ['server', 'app', 'index', 'main'];
    const mainFiles: string[] = [];
    
    for (const file of files) {
      const fileName = file.split('/').pop()?.split('.')[0]?.toLowerCase();
      
      if (fileName && mainFileNames.includes(fileName)) {
        // Prioritize files in root or src directory
        if (file.includes('/src/') || !file.includes('/')) {
          mainFiles.unshift(file); // Add to beginning
        } else {
          mainFiles.push(file);
        }
      }
    }
    
    return mainFiles;
  }

  /**
   * File system utility methods
   */
  private findFiles(patterns: string[]): string[] {
    const files: string[] = [];
    for (const pattern of patterns) {
      try {
        const matches = glob.sync(pattern, { cwd: this.projectRoot, absolute: true });
        files.push(...matches);
      } catch (error) {
        // Ignore glob errors, continue with other patterns
      }
    }
    return [...new Set(files)]; // Remove duplicates
  }

  private readFileContent(filePath: string): string {
    try {
      return readFileSync(filePath, 'utf8');
    } catch {
      return '';
    }
  }

  private readPackageJson(): any {
    try {
      const pkgPath = join(this.projectRoot, 'package.json');
      return JSON.parse(readFileSync(pkgPath, 'utf8'));
    } catch {
      return {};
    }
  }

  private determineProjectType(pkg: any, mainFiles: string[] = []): string {
    // Check package.json dependencies first
    if (pkg.dependencies?.express || pkg.devDependencies?.express) return 'Express API';
    if (pkg.dependencies?.react || pkg.devDependencies?.react) return 'React Application';
    if (pkg.dependencies?.['@nestjs/core']) return 'NestJS API';
    if (pkg.dependencies?.next || pkg.devDependencies?.next) return 'Next.js Application';
    if (pkg.dependencies?.vue || pkg.devDependencies?.vue) return 'Vue Application';
    if (pkg.dependencies?.fastify || pkg.devDependencies?.fastify) return 'Fastify API';
    if (pkg.dependencies?.koa || pkg.devDependencies?.koa) return 'Koa API';
    
    // Analyze main files for additional clues
    for (const file of mainFiles) {
      const content = this.readFileContent(file);
      if (content.includes('express()') || content.includes('app.listen') || content.includes('app.use')) {
        return 'Express API';
      }
      if (content.includes('fastify') || content.includes('fastify()')) {
        return 'Fastify API';  
      }
      if (content.includes('new Koa') || content.includes('koa')) {
        return 'Koa API';
      }
      if (content.includes('createServer') || content.includes('http.')) {
        return 'HTTP Server';
      }
    }
    
    return 'Node.js Project';
  }

  private determineLanguage(pkg: any): string {
    if (pkg.devDependencies?.typescript || existsSync(join(this.projectRoot, 'tsconfig.json'))) {
      return 'TypeScript';
    }
    return 'JavaScript';
  }

  private determineFramework(pkg: any, mainFiles: string[] = []): string {
    // Check package.json dependencies first
    if (pkg.dependencies?.express) return 'Express.js';
    if (pkg.dependencies?.['@nestjs/core']) return 'NestJS';
    if (pkg.dependencies?.react) return 'React';
    if (pkg.dependencies?.next) return 'Next.js';
    if (pkg.dependencies?.vue) return 'Vue.js';
    if (pkg.dependencies?.fastify) return 'Fastify';
    if (pkg.dependencies?.koa) return 'Koa';
    
    // Analyze main files for framework usage patterns
    for (const file of mainFiles) {
      const content = this.readFileContent(file);
      if (content.includes('express()') || content.includes('require("express")') || content.includes('import express')) {
        return 'Express.js';
      }
      if (content.includes('fastify') || content.includes('require("fastify")') || content.includes('import fastify')) {
        return 'Fastify';
      }
      if (content.includes('new Koa') || content.includes('require("koa")') || content.includes('import Koa')) {
        return 'Koa';
      }
    }
    
    return 'None detected';
  }

  private determinePackageManager(): string {
    if (existsSync(join(this.projectRoot, 'pnpm-lock.yaml'))) return 'pnpm';
    if (existsSync(join(this.projectRoot, 'yarn.lock'))) return 'yarn';
    if (existsSync(join(this.projectRoot, 'package-lock.json'))) return 'npm';
    return 'npm';
  }

  private mapKeyDirectories(): Array<{path: string, purpose: string}> {
    const dirs = [];
    const checkDirs = [
      { path: 'src', purpose: 'source code' },
      { path: 'src/components', purpose: 'React components' },
      { path: 'src/services', purpose: 'business logic' },
      { path: 'src/middleware', purpose: 'middleware functions' },
      { path: 'src/controllers', purpose: 'API controllers' },
      { path: 'src/routes', purpose: 'API routes' },
      { path: 'src/models', purpose: 'data models' },
      { path: 'src/utils', purpose: 'utility functions' },
      { path: 'src/types', purpose: 'TypeScript types' },
      { path: 'tests', purpose: 'test files' },
      { path: '__tests__', purpose: 'test files' }
    ];

    for (const dir of checkDirs) {
      const fullPath = join(this.projectRoot, dir.path);
      if (existsSync(fullPath) && statSync(fullPath).isDirectory()) {
        dirs.push(dir);
      }
    }

    return dirs;
  }
}