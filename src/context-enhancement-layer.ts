import * as fs from 'fs';
import * as path from 'path';
import { CodeChunk, QueryRequest, MultiHopConfig } from './types';
import { log, warn, error } from './logging-utils';
import { ProjectManager } from './project-manager';
import { CodebaseIndexer } from './indexer';

interface ProjectContext {
  type: 'typescript' | 'javascript' | 'python' | 'react' | 'nextjs' | 'express' | 'fastapi' | 'unknown';
  structure: {
    hasSource: boolean;
    sourceDir: string;
    hasTests: boolean;
    testDir: string;
    configFiles: string[];
    mainEntries: string[];
  };
  libraries: {
    dependencies: string[];
    devDependencies: string[];
    detectedFrameworks: string[];
  };
  metadata: {
    name: string;
    version: string;
    description: string;
    language: string;
  };
}

interface ContextEnhancement {
  projectAwareness: string;
  architecturalContext: string;
  libraryContext: string;
  tokensAdded: number;
}

interface EnhancedSemanticResult {
  results: string;
  enhancement: ContextEnhancement;
  stats: {
    enhanced: boolean;
    tokensAdded: number;
    contextAccuracy: number;
  };
}

/**
 * Context Enhancement Layer for Cortex V3.0
 * 
 * Automatically injects project awareness into semantic search results
 * to improve Claude Code suggestion accuracy and relevance
 */
export class ContextEnhancementLayer {
  private projectContextCache: Map<string, ProjectContext> = new Map();
  private projectManager?: ProjectManager;
  private indexer?: CodebaseIndexer;

  constructor() {
    // Context cache expires after 5 minutes to detect project changes
    setInterval(() => {
      this.cleanupStaleContexts();
    }, 5 * 60 * 1000);
  }

  async initialize(): Promise<void> {
    try {
      log('[ContextEnhancer] Initializing project context detection...');
      
      // Initialize project manager if needed
      this.projectManager = new ProjectManager();
      
      log('[ContextEnhancer] Context enhancement layer ready');
    } catch (error) {
      console.error(`[ContextEnhancer] Failed to initialize: ${(error as Error).message}`);
      throw error;
    }
  }

  /**
   * Enhance semantic search results with project context
   */
  async enhanceSemanticSearch(
    query: string,
    chunks: CodeChunk[],
    projectPath: string,
    options?: { maxTokens?: number }
  ): Promise<EnhancedSemanticResult> {
    try {
      const startTime = Date.now();
      
      // Get project context
      const projectContext = await this.getProjectContext(projectPath);
      
      // Generate context enhancement
      const enhancement = await this.generateContextEnhancement(
        query,
        chunks,
        projectContext,
        options?.maxTokens || 150
      );
      
      // Apply enhancement to results
      const enhancedResults = this.applyContextEnhancement(chunks, enhancement);
      
      const processingTime = Date.now() - startTime;
      
      return {
        results: enhancedResults,
        enhancement,
        stats: {
          enhanced: true,
          tokensAdded: enhancement.tokensAdded,
          contextAccuracy: this.calculateContextAccuracy(projectContext, chunks)
        }
      };
      
    } catch (error) {
      console.error(`[ContextEnhancer] Enhancement failed: ${(error as Error).message}`);
      
      // Fallback: return original results without enhancement
      return {
        results: this.formatChunksAsResults(chunks),
        enhancement: {
          projectAwareness: '',
          architecturalContext: '',
          libraryContext: '',
          tokensAdded: 0
        },
        stats: {
          enhanced: false,
          tokensAdded: 0,
          contextAccuracy: 0
        }
      };
    }
  }

  /**
   * Enhance code intelligence requests with project context
   */
  async enhanceCodeIntelligence(
    request: QueryRequest,
    projectPath: string
  ): Promise<any> {
    try {
      const projectContext = await this.getProjectContext(projectPath);
      
      // Add project context to the request
      const enhancedRequest = {
        ...request,
        context: {
          project: projectContext,
          taskContext: this.generateTaskContext(request.task, projectContext)
        }
      };
      
      return {
        request: enhancedRequest,
        contextInjected: true,
        projectType: projectContext.type,
        detectedFrameworks: projectContext.libraries.detectedFrameworks
      };
      
    } catch (error) {
      console.error(`[ContextEnhancer] Code intelligence enhancement failed: ${(error as Error).message}`);
      return {
        request,
        contextInjected: false,
        error: (error as Error).message
      };
    }
  }

  /**
   * Enhance relationship analysis with project context
   */
  async enhanceRelationshipAnalysis(
    request: any,
    projectPath: string
  ): Promise<any> {
    try {
      const projectContext = await this.getProjectContext(projectPath);
      
      // Enhance relationship analysis with project structure awareness
      const enhancedRequest = {
        ...request,
        projectContext: {
          type: projectContext.type,
          structure: projectContext.structure,
          libraries: projectContext.libraries.detectedFrameworks
        }
      };
      
      return {
        request: enhancedRequest,
        contextEnhanced: true,
        projectStructure: projectContext.structure
      };
      
    } catch (error) {
      console.error(`[ContextEnhancer] Relationship analysis enhancement failed: ${(error as Error).message}`);
      return {
        request,
        contextEnhanced: false,
        error: (error as Error).message
      };
    }
  }

  /**
   * Get or generate project context for a given project path
   */
  private async getProjectContext(projectPath: string): Promise<ProjectContext> {
    // Check cache first
    const cached = this.projectContextCache.get(projectPath);
    if (cached) {
      return cached;
    }

    // Generate new project context
    const context = await this.analyzeProject(projectPath);
    
    // Cache for future use
    this.projectContextCache.set(projectPath, context);
    
    return context;
  }

  /**
   * Analyze project structure and detect context
   */
  private async analyzeProject(projectPath: string): Promise<ProjectContext> {
    try {
      // Check if project path exists
      if (!fs.existsSync(projectPath)) {
        warn(`[ContextEnhancer] Project path does not exist: ${projectPath}`);
        return this.getDefaultContext();
      }

      // Read package.json if exists
      const packageJsonPath = path.join(projectPath, 'package.json');
      let packageJson: any = {};
      
      if (fs.existsSync(packageJsonPath)) {
        try {
          const content = fs.readFileSync(packageJsonPath, 'utf-8');
          packageJson = JSON.parse(content);
        } catch (error) {
          warn(`[ContextEnhancer] Failed to parse package.json: ${(error as Error).message}`);
        }
      }

      // Analyze project structure
      const structure = await this.analyzeProjectStructure(projectPath);
      
      // Detect project type
      const type = this.detectProjectType(projectPath, packageJson, structure);
      
      // Extract library information
      const libraries = this.extractLibraryInfo(packageJson);
      
      // Generate metadata
      const metadata = {
        name: packageJson.name || path.basename(projectPath),
        version: packageJson.version || '1.0.0',
        description: packageJson.description || '',
        language: this.detectPrimaryLanguage(structure)
      };

      return {
        type,
        structure,
        libraries,
        metadata
      };

    } catch (error) {
      console.error(`[ContextEnhancer] Project analysis failed: ${(error as Error).message}`);
      return this.getDefaultContext();
    }
  }

  /**
   * Analyze project directory structure
   */
  private async analyzeProjectStructure(projectPath: string): Promise<ProjectContext['structure']> {
    const structure = {
      hasSource: false,
      sourceDir: 'src',
      hasTests: false,
      testDir: 'tests',
      configFiles: [] as string[],
      mainEntries: [] as string[]
    };

    try {
      const entries = fs.readdirSync(projectPath);
      
      // Check for source directories
      const sourceDirs = ['src', 'lib', 'app', 'components'];
      for (const dir of sourceDirs) {
        if (entries.includes(dir) && fs.statSync(path.join(projectPath, dir)).isDirectory()) {
          structure.hasSource = true;
          structure.sourceDir = dir;
          break;
        }
      }

      // Check for test directories
      const testDirs = ['tests', 'test', '__tests__', 'spec'];
      for (const dir of testDirs) {
        if (entries.includes(dir) && fs.statSync(path.join(projectPath, dir)).isDirectory()) {
          structure.hasTests = true;
          structure.testDir = dir;
          break;
        }
      }

      // Identify config files
      const configPatterns = [
        'tsconfig.json', 'jsconfig.json', 'webpack.config.js', 'vite.config.js',
        'next.config.js', 'babel.config.js', '.eslintrc', 'prettier.config.js',
        'tailwind.config.js', 'jest.config.js', 'cypress.config.js'
      ];
      
      for (const pattern of configPatterns) {
        if (entries.includes(pattern)) {
          structure.configFiles.push(pattern);
        }
      }

      // Identify main entry files
      const entryFiles = ['index.ts', 'index.js', 'main.ts', 'main.js', 'app.ts', 'app.js'];
      for (const file of entryFiles) {
        if (entries.includes(file)) {
          structure.mainEntries.push(file);
        }
      }

      return structure;

    } catch (error) {
      warn(`[ContextEnhancer] Structure analysis failed: ${(error as Error).message}`);
      return structure;
    }
  }

  /**
   * Detect project type based on structure and dependencies
   */
  private detectProjectType(
    projectPath: string,
    packageJson: any,
    structure: ProjectContext['structure']
  ): ProjectContext['type'] {
    // Check for specific frameworks first
    const deps = {
      ...packageJson.dependencies,
      ...packageJson.devDependencies
    };

    // Next.js detection
    if (deps['next'] || structure.configFiles.includes('next.config.js')) {
      return 'nextjs';
    }

    // React detection
    if (deps['react'] || deps['@types/react']) {
      return 'react';
    }

    // Express detection
    if (deps['express'] || deps['@types/express']) {
      return 'express';
    }

    // FastAPI detection (Python)
    const requirementsTxt = path.join(projectPath, 'requirements.txt');
    const pyprojectToml = path.join(projectPath, 'pyproject.toml');
    if (fs.existsSync(requirementsTxt) || fs.existsSync(pyprojectToml)) {
      return 'python';
    }

    // TypeScript detection
    if (structure.configFiles.includes('tsconfig.json') || 
        deps['typescript'] || 
        deps['@types/node']) {
      return 'typescript';
    }

    // JavaScript detection
    if (packageJson.name || structure.mainEntries.some(f => f.endsWith('.js'))) {
      return 'javascript';
    }

    return 'unknown';
  }

  /**
   * Extract library and framework information
   */
  private extractLibraryInfo(packageJson: any): ProjectContext['libraries'] {
    const dependencies = Object.keys(packageJson.dependencies || {});
    const devDependencies = Object.keys(packageJson.devDependencies || {});
    
    // Detect major frameworks and libraries
    const frameworks: string[] = [];
    
    const frameworkPatterns = {
      'React': ['react', '@types/react'],
      'Next.js': ['next'],
      'Express': ['express', '@types/express'],
      'Vue': ['vue', '@vue/core'],
      'Angular': ['@angular/core', '@angular/cli'],
      'Svelte': ['svelte'],
      'FastAPI': ['fastapi'],
      'Django': ['django'],
      'Flask': ['flask']
    };

    for (const [framework, patterns] of Object.entries(frameworkPatterns)) {
      if (patterns.some(pattern => dependencies.includes(pattern) || devDependencies.includes(pattern))) {
        frameworks.push(framework);
      }
    }

    return {
      dependencies,
      devDependencies,
      detectedFrameworks: frameworks
    };
  }

  /**
   * Detect primary programming language
   */
  private detectPrimaryLanguage(structure: ProjectContext['structure']): string {
    if (structure.configFiles.includes('tsconfig.json')) return 'TypeScript';
    if (structure.mainEntries.some(f => f.endsWith('.js'))) return 'JavaScript';
    if (structure.configFiles.some(f => f.includes('pyproject') || f.includes('requirements'))) return 'Python';
    return 'JavaScript'; // Default fallback
  }

  /**
   * Generate context enhancement text
   */
  private async generateContextEnhancement(
    query: string,
    chunks: CodeChunk[],
    projectContext: ProjectContext,
    maxTokens: number
  ): Promise<ContextEnhancement> {
    let tokensUsed = 0;
    const maxTokensPerSection = Math.floor(maxTokens / 3);

    // Project awareness section
    let projectAwareness = '';
    if (tokensUsed < maxTokens) {
      projectAwareness = this.generateProjectAwareness(projectContext);
      tokensUsed += this.estimateTokens(projectAwareness);
    }

    // Architectural context section
    let architecturalContext = '';
    if (tokensUsed < maxTokens) {
      architecturalContext = this.generateArchitecturalContext(projectContext, chunks);
      const contextTokens = this.estimateTokens(architecturalContext);
      if (tokensUsed + contextTokens <= maxTokens) {
        tokensUsed += contextTokens;
      } else {
        architecturalContext = '';
      }
    }

    // Library context section
    let libraryContext = '';
    if (tokensUsed < maxTokens) {
      libraryContext = this.generateLibraryContext(projectContext, query);
      const libTokens = this.estimateTokens(libraryContext);
      if (tokensUsed + libTokens <= maxTokens) {
        tokensUsed += libTokens;
      } else {
        libraryContext = '';
      }
    }

    return {
      projectAwareness,
      architecturalContext,
      libraryContext,
      tokensAdded: tokensUsed
    };
  }

  /**
   * Generate project awareness text
   */
  private generateProjectAwareness(projectContext: ProjectContext): string {
    if (projectContext.type === 'unknown') return '';

    const frameworks = projectContext.libraries.detectedFrameworks.join(', ');
    const structure = projectContext.structure;

    return `PROJECT: ${projectContext.metadata.name} (${projectContext.type})
STRUCTURE: ${structure.sourceDir}/, ${structure.hasTests ? structure.testDir + '/' : 'no tests'}, ${structure.configFiles.slice(0, 2).join(', ')}
LIBRARIES: ${frameworks || 'vanilla ' + projectContext.metadata.language}`;
  }

  /**
   * Generate architectural context text
   */
  private generateArchitecturalContext(projectContext: ProjectContext, chunks: CodeChunk[]): string {
    if (chunks.length === 0) return '';

    // Analyze chunk patterns
    const chunkTypes = chunks.reduce((acc, chunk) => {
      acc[chunk.chunk_type] = (acc[chunk.chunk_type] || 0) + 1;
      return acc;
    }, {} as Record<string, number>);

    const patterns = Object.entries(chunkTypes)
      .sort(([,a], [,b]) => b - a)
      .slice(0, 3)
      .map(([type, count]) => `${count} ${type}${count > 1 ? 's' : ''}`)
      .join(', ');

    return patterns ? `PATTERNS: ${patterns}` : '';
  }

  /**
   * Generate library-specific context
   */
  private generateLibraryContext(projectContext: ProjectContext, query: string): string {
    const frameworks = projectContext.libraries.detectedFrameworks;
    if (frameworks.length === 0) return '';

    // Generate context based on query and frameworks
    const contextHints: string[] = [];

    if (frameworks.includes('React') && query.toLowerCase().includes('component')) {
      contextHints.push('React functional components with hooks');
    }

    if (frameworks.includes('Express') && query.toLowerCase().includes('auth')) {
      contextHints.push('Express middleware patterns');
    }

    if (frameworks.includes('Next.js') && query.toLowerCase().includes('api')) {
      contextHints.push('Next.js API routes');
    }

    if (projectContext.type === 'typescript') {
      contextHints.push('TypeScript type definitions');
    }

    return contextHints.length > 0 ? `CONTEXT: ${contextHints.join(', ')}` : '';
  }

  /**
   * Apply context enhancement to search results
   */
  private applyContextEnhancement(
    chunks: CodeChunk[],
    enhancement: ContextEnhancement
  ): string {
    const enhancementHeader = [
      enhancement.projectAwareness,
      enhancement.architecturalContext,
      enhancement.libraryContext
    ].filter(Boolean).join('\n');

    const results = this.formatChunksAsResults(chunks);
    
    return enhancementHeader 
      ? `${enhancementHeader}\n\n## Semantic Search Results\n${results}`
      : results;
  }

  /**
   * Format chunks as search results
   */
  private formatChunksAsResults(chunks: CodeChunk[]): string {
    if (chunks.length === 0) {
      return 'No matching code chunks found.';
    }

    return chunks
      .slice(0, 10) // Limit to top 10 results
      .map((chunk, index) => {
        const location = `${chunk.file_path}:${chunk.start_line}`;
        const relevance = chunk.relevance_score 
          ? ` (${Math.round(chunk.relevance_score * 100)}% match)`
          : '';
        
        return `## ${chunk.chunk_type} (${location})${relevance}
\`\`\`${chunk.language_metadata.language}
${chunk.content.trim()}
\`\`\``;
      })
      .join('\n\n');
  }

  /**
   * Generate task-specific context
   */
  private generateTaskContext(task: string, projectContext: ProjectContext): string {
    const taskLower = task.toLowerCase();
    
    // Authentication context
    if (taskLower.includes('auth') || taskLower.includes('login') || taskLower.includes('jwt')) {
      if (projectContext.libraries.detectedFrameworks.includes('Express')) {
        return 'Express authentication middleware and JWT token handling patterns';
      }
      if (projectContext.libraries.detectedFrameworks.includes('React')) {
        return 'React authentication state management and protected routes';
      }
    }

    // API context
    if (taskLower.includes('api') || taskLower.includes('endpoint') || taskLower.includes('route')) {
      if (projectContext.libraries.detectedFrameworks.includes('Next.js')) {
        return 'Next.js API routes and serverless functions';
      }
      if (projectContext.libraries.detectedFrameworks.includes('Express')) {
        return 'Express route handlers and REST API patterns';
      }
    }

    // Component context
    if (taskLower.includes('component') || taskLower.includes('ui')) {
      if (projectContext.libraries.detectedFrameworks.includes('React')) {
        return 'React component composition and prop patterns';
      }
    }

    return `${projectContext.type} development patterns`;
  }

  /**
   * Calculate context accuracy score
   */
  private calculateContextAccuracy(projectContext: ProjectContext, chunks: CodeChunk[]): number {
    if (projectContext.type === 'unknown' || chunks.length === 0) {
      return 0;
    }

    // Score based on project detection confidence
    let accuracy = 0.5; // Base score

    // Boost for detected frameworks
    if (projectContext.libraries.detectedFrameworks.length > 0) {
      accuracy += 0.3;
    }

    // Boost for structural analysis
    if (projectContext.structure.hasSource) {
      accuracy += 0.1;
    }

    // Boost for config files detected
    if (projectContext.structure.configFiles.length > 0) {
      accuracy += 0.1;
    }

    return Math.min(1.0, accuracy);
  }

  /**
   * Estimate token count for text (rough approximation)
   */
  private estimateTokens(text: string): number {
    // Rough estimation: ~4 characters per token
    return Math.ceil(text.length / 4);
  }

  /**
   * Get default context for unknown projects
   */
  private getDefaultContext(): ProjectContext {
    return {
      type: 'unknown',
      structure: {
        hasSource: false,
        sourceDir: 'src',
        hasTests: false,
        testDir: 'tests',
        configFiles: [],
        mainEntries: []
      },
      libraries: {
        dependencies: [],
        devDependencies: [],
        detectedFrameworks: []
      },
      metadata: {
        name: 'unknown',
        version: '1.0.0',
        description: '',
        language: 'JavaScript'
      }
    };
  }

  /**
   * Clean up stale project contexts (older than 5 minutes)
   */
  private cleanupStaleContexts(): void {
    const staleThreshold = Date.now() - (5 * 60 * 1000);
    
    // For now, we don't track timestamps, so just clear cache periodically
    if (this.projectContextCache.size > 50) { // Arbitrary limit
      this.projectContextCache.clear();
      log('[ContextEnhancer] Cleared project context cache');
    }
  }
}