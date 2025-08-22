import * as fs from 'fs';
import * as path from 'path';
import { log, warn } from './logging-utils';

export interface ProjectContext {
  type: string;
  language: string;
  framework: string;
  packageManager: string;
  directories: string[];
  dependencies: string[];
  patterns: {
    auth?: string;
    database?: string;
    testing?: string;
    validation?: string;
  };
  lastUpdated: string;
  confidence: number;
}

interface PackageJson {
  name?: string;
  dependencies?: Record<string, string>;
  devDependencies?: Record<string, string>;
  scripts?: Record<string, string>;
}

interface TSConfig {
  compilerOptions?: {
    strict?: boolean;
    baseUrl?: string;
    paths?: Record<string, string[]>;
  };
}

/**
 * Project Context Detector for Cortex V3.0
 * 
 * Detects essential project information to enhance Claude Code's context awareness:
 * - Project type (Express API, React App, FastAPI, etc.)
 * - Language and framework details
 * - Key directory structure
 * - Core dependencies and patterns
 */
export class ProjectContextDetector {
  private contextCache: Map<string, { context: ProjectContext; timestamp: number }> = new Map();
  private readonly cacheTimeout = 5 * 60 * 1000; // 5 minutes

  /**
   * Detect project context for a given project path
   * Uses caching to avoid repeated file system operations
   */
  async detectProjectContext(projectPath: string): Promise<ProjectContext> {
    const cached = this.contextCache.get(projectPath);
    
    if (cached && Date.now() - cached.timestamp < this.cacheTimeout) {
      return cached.context;
    }

    const context = await this.performDetection(projectPath);
    
    this.contextCache.set(projectPath, {
      context,
      timestamp: Date.now()
    });

    return context;
  }

  private async performDetection(projectPath: string): Promise<ProjectContext> {
    try {
      const pkg = await this.readPackageJson(projectPath);
      const tsconfig = await this.readTSConfig(projectPath);
      const directories = await this.scanKeyDirectories(projectPath);

      const projectType = this.determineProjectType(pkg, directories);
      const language = this.determineLanguage(pkg, tsconfig, projectPath);
      const framework = this.extractFramework(pkg);
      const packageManager = this.detectPackageManager(projectPath);
      const dependencies = this.extractCoreDependencies(pkg);
      const patterns = this.detectPatterns(pkg, directories);

      const confidence = this.calculateConfidence(pkg, tsconfig, directories);

      return {
        type: projectType,
        language,
        framework,
        packageManager,
        directories: directories.slice(0, 4), // Limit to top 4 directories
        dependencies: dependencies.slice(0, 6), // Limit to top 6 dependencies
        patterns,
        lastUpdated: new Date().toISOString(),
        confidence
      };

    } catch (error) {
      warn('Project context detection failed', { projectPath, error });
      
      return {
        type: 'Unknown Project',
        language: 'unknown',
        framework: 'unknown',
        packageManager: 'unknown',
        directories: [],
        dependencies: [],
        patterns: {},
        lastUpdated: new Date().toISOString(),
        confidence: 0.0
      };
    }
  }

  private async readPackageJson(projectPath: string): Promise<PackageJson | null> {
    try {
      const packagePath = path.join(projectPath, 'package.json');
      if (!fs.existsSync(packagePath)) return null;
      
      const content = fs.readFileSync(packagePath, 'utf-8');
      return JSON.parse(content);
    } catch {
      return null;
    }
  }

  private async readTSConfig(projectPath: string): Promise<TSConfig | null> {
    try {
      const tsconfigPath = path.join(projectPath, 'tsconfig.json');
      if (!fs.existsSync(tsconfigPath)) return null;
      
      const content = fs.readFileSync(tsconfigPath, 'utf-8');
      return JSON.parse(content);
    } catch {
      return null;
    }
  }

  private async scanKeyDirectories(projectPath: string): Promise<string[]> {
    const commonDirectories = [
      // JavaScript/TypeScript patterns
      'src/services', 'src/controllers', 'src/routes', 'src/middleware',
      'src/types', 'src/interfaces', 'src/models', 'src/utils', 'src/helpers',
      'src/components', 'src/hooks', 'src/pages', 'src/stores', 'src/contexts',
      
      // Alternative patterns  
      'lib', 'app', 'api', 'server', 'client', 'public', 'static',
      
      // Configuration and tooling
      'config', 'scripts', 'tests', '__tests__', 'test', 'spec',
      
      // Framework specific
      'pages', 'components', 'layouts', 'plugins', 'middleware',
      
      // Python patterns
      'models', 'views', 'serializers', 'migrations', 'management',
      'templates', 'static', 'media'
    ];

    const foundDirectories: string[] = [];

    for (const dir of commonDirectories) {
      const fullPath = path.join(projectPath, dir);
      if (fs.existsSync(fullPath) && fs.statSync(fullPath).isDirectory()) {
        foundDirectories.push(dir);
      }
    }

    return foundDirectories;
  }

  private determineProjectType(pkg: PackageJson | null, directories: string[]): string {
    if (!pkg) {
      // Fallback to directory-based detection
      if (directories.some(d => d.includes('components'))) return 'React Application';
      if (directories.some(d => d.includes('services'))) return 'Backend API';
      return 'Unknown Project';
    }

    const allDeps = { ...pkg.dependencies, ...pkg.devDependencies };

    // Express.js detection
    if (allDeps.express) {
      if (allDeps.typescript || allDeps['@types/express']) {
        return 'Express TypeScript API';
      }
      return 'Express.js API';
    }

    // React detection
    if (allDeps.react) {
      if (allDeps['next']) return 'Next.js Application';
      if (allDeps['@remix-run/react']) return 'Remix Application';
      return 'React Application';
    }

    // Vue detection
    if (allDeps.vue) {
      if (allDeps.nuxt) return 'Nuxt.js Application';
      return 'Vue.js Application';
    }

    // Node.js backend frameworks
    if (allDeps.fastify) return 'Fastify API';
    if (allDeps.koa) return 'Koa.js API';
    if (allDeps['@nestjs/core']) return 'NestJS Application';

    // Python detection (if package.json exists alongside Python files)
    if (this.fileExists(path.dirname(pkg.name || ''), 'requirements.txt') ||
        this.fileExists(path.dirname(pkg.name || ''), 'pyproject.toml')) {
      return 'Python + Node.js Project';
    }

    // Generic Node.js project
    if (Object.keys(allDeps).length > 0) {
      return 'Node.js Project';
    }

    return 'Unknown Project';
  }

  private determineLanguage(pkg: PackageJson | null, tsconfig: TSConfig | null, projectPath: string): string {
    // TypeScript detection
    if (tsconfig) return 'TypeScript';
    if (pkg?.devDependencies?.typescript || pkg?.dependencies?.typescript) return 'TypeScript';
    
    // Check for .ts files in src directory
    const srcPath = path.join(projectPath, 'src');
    if (fs.existsSync(srcPath)) {
      const files = fs.readdirSync(srcPath);
      if (files.some(f => f.endsWith('.ts') || f.endsWith('.tsx'))) {
        return 'TypeScript';
      }
    }

    // Python detection
    if (this.fileExists(projectPath, 'requirements.txt') ||
        this.fileExists(projectPath, 'pyproject.toml') ||
        this.fileExists(projectPath, 'setup.py')) {
      return 'Python';
    }

    // Go detection
    if (this.fileExists(projectPath, 'go.mod')) return 'Go';

    // Java detection
    if (this.fileExists(projectPath, 'pom.xml') || 
        this.fileExists(projectPath, 'build.gradle')) return 'Java';

    // Default to JavaScript if package.json exists
    if (pkg) return 'JavaScript';

    return 'Unknown';
  }

  private extractFramework(pkg: PackageJson | null): string {
    if (!pkg) return 'unknown';

    const allDeps = { ...pkg.dependencies, ...pkg.devDependencies };

    // Primary frameworks
    if (allDeps.express) return 'Express.js';
    if (allDeps.react) return 'React';
    if (allDeps.vue) return 'Vue.js';
    if (allDeps.angular) return 'Angular';
    if (allDeps.svelte) return 'Svelte';
    if (allDeps.fastify) return 'Fastify';
    if (allDeps.koa) return 'Koa.js';
    if (allDeps['@nestjs/core']) return 'NestJS';

    // Meta-frameworks
    if (allDeps.next) return 'Next.js';
    if (allDeps.nuxt) return 'Nuxt.js';
    if (allDeps.gatsby) return 'Gatsby';
    if (allDeps['@remix-run/react']) return 'Remix';

    return 'Node.js';
  }

  private detectPackageManager(projectPath: string): string {
    if (this.fileExists(projectPath, 'yarn.lock')) return 'Yarn';
    if (this.fileExists(projectPath, 'pnpm-lock.yaml')) return 'PNPM';
    if (this.fileExists(projectPath, 'package-lock.json')) return 'NPM';
    if (this.fileExists(projectPath, 'bun.lockb')) return 'Bun';
    return 'unknown';
  }

  private extractCoreDependencies(pkg: PackageJson | null): string[] {
    if (!pkg) return [];

    const allDeps = { ...pkg.dependencies, ...pkg.devDependencies };
    
    const importantLibraries = [
      // Core frameworks
      'express', 'react', 'vue', 'angular', 'svelte', 'fastify', 'koa',
      
      // Meta-frameworks  
      'next', 'nuxt', 'gatsby', '@nestjs/core', '@remix-run/react',
      
      // Languages and build tools
      'typescript', '@types/node', 'babel', 'webpack', 'vite', 'rollup',
      
      // Databases and ORMs
      'prisma', 'mongoose', 'typeorm', 'sequelize', 'knex',
      
      // Authentication and security
      'jsonwebtoken', 'passport', 'bcrypt', 'helmet', 'cors',
      
      // Validation
      'zod', 'joi', 'yup', 'ajv',
      
      // Testing
      'jest', 'mocha', 'vitest', 'cypress', '@testing-library/react',
      
      // Utilities
      'lodash', 'axios', 'dotenv', 'moment', 'dayjs'
    ];

    return importantLibraries.filter(lib => allDeps[lib]);
  }

  private detectPatterns(pkg: PackageJson | null, directories: string[]): ProjectContext['patterns'] {
    const patterns: ProjectContext['patterns'] = {};

    if (!pkg) return patterns;

    const allDeps = { ...pkg.dependencies, ...pkg.devDependencies };

    // Authentication patterns
    if (allDeps.jsonwebtoken) patterns.auth = 'JWT tokens';
    else if (allDeps.passport) patterns.auth = 'Passport.js';
    else if (allDeps['@auth0/nextjs-auth0']) patterns.auth = 'Auth0';
    else if (directories.some(d => d.includes('middleware') || d.includes('auth'))) {
      patterns.auth = 'Custom middleware';
    }

    // Database patterns
    if (allDeps.prisma) patterns.database = 'Prisma ORM';
    else if (allDeps.mongoose) patterns.database = 'Mongoose (MongoDB)';
    else if (allDeps.typeorm) patterns.database = 'TypeORM';
    else if (allDeps.sequelize) patterns.database = 'Sequelize';
    else if (allDeps.knex) patterns.database = 'Knex.js';

    // Testing patterns
    if (allDeps.jest) patterns.testing = 'Jest';
    else if (allDeps.mocha) patterns.testing = 'Mocha';
    else if (allDeps.vitest) patterns.testing = 'Vitest';
    else if (allDeps.cypress) patterns.testing = 'Cypress';

    // Validation patterns
    if (allDeps.zod) patterns.validation = 'Zod schemas';
    else if (allDeps.joi) patterns.validation = 'Joi validation';
    else if (allDeps.yup) patterns.validation = 'Yup schemas';

    return patterns;
  }

  private calculateConfidence(pkg: PackageJson | null, tsconfig: TSConfig | null, directories: string[]): number {
    let confidence = 0.0;

    // Package.json presence and content
    if (pkg) {
      confidence += 0.3;
      if (Object.keys(pkg.dependencies || {}).length > 0) confidence += 0.2;
      if (Object.keys(pkg.devDependencies || {}).length > 0) confidence += 0.1;
    }

    // TypeScript configuration
    if (tsconfig) confidence += 0.2;

    // Directory structure
    if (directories.length > 0) confidence += 0.1;
    if (directories.length >= 3) confidence += 0.1;

    return Math.min(confidence, 1.0);
  }

  private fileExists(projectPath: string, filename: string): boolean {
    return fs.existsSync(path.join(projectPath, filename));
  }

  /**
   * Invalidate cache for a specific project (useful for file watching integration)
   */
  invalidateCache(projectPath: string): void {
    this.contextCache.delete(projectPath);
    log('Project context cache invalidated', { projectPath });
  }

  /**
   * Clear all cached contexts
   */
  clearCache(): void {
    this.contextCache.clear();
    log('All project context cache cleared');
  }

  /**
   * Get cache statistics
   */
  getCacheStats(): { entries: number; hitRate: number } {
    return {
      entries: this.contextCache.size,
      hitRate: 0 // TODO: Implement hit rate tracking
    };
  }
}