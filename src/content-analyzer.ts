import * as fs from 'fs/promises';
import * as path from 'path';
import { log, warn } from './logging-utils';

export interface ContentAnalysis {
  language: string;
  complexity: number;
  hasImports: boolean;
  hasExports: boolean;
  hasTests: boolean;
  hasDocumentation: boolean;
  codeToCommentRatio: number;
  uniqueTokens: number;
  semanticValue: 'high' | 'medium' | 'low';
  fileType: 'source' | 'config' | 'documentation' | 'test' | 'build' | 'data';
  estimatedImportance: number; // 0-100
}

export interface LanguagePattern {
  extensions: string[];
  importPatterns: RegExp[];
  exportPatterns: RegExp[];
  commentPatterns: RegExp[];
  complexityIndicators: RegExp[];
  testPatterns: RegExp[];
}

export class ContentAnalyzer {
  private languagePatterns: Map<string, LanguagePattern> = new Map();
  private fileTypeCache: Map<string, string> = new Map();

  constructor() {
    this.initializeLanguagePatterns();
  }

  async analyzeFile(filePath: string): Promise<ContentAnalysis> {
    try {
      const content = await fs.readFile(filePath, 'utf-8');
      const stats = await fs.stat(filePath);
      
      // Skip very large files to avoid performance issues
      if (stats.size > 1024 * 1024) { // 1MB limit
        return this.createMinimalAnalysis(filePath, 'File too large');
      }

      const language = this.detectLanguage(filePath, content);
      const analysis: ContentAnalysis = {
        language,
        complexity: this.calculateComplexity(content, language),
        hasImports: this.hasImports(content, language),
        hasExports: this.hasExports(content, language),
        hasTests: this.hasTests(content, language),
        hasDocumentation: this.hasDocumentation(content),
        codeToCommentRatio: this.calculateCodeCommentRatio(content, language),
        uniqueTokens: this.countUniqueTokens(content),
        semanticValue: 'medium', // Will be calculated below
        fileType: this.determineFileType(filePath, content),
        estimatedImportance: 0 // Will be calculated below
      };

      // Calculate semantic value and importance
      analysis.semanticValue = this.assessSemanticValue(analysis, content);
      analysis.estimatedImportance = this.calculateImportance(analysis, filePath);

      return analysis;
    } catch (error) {
      warn(`[ContentAnalyzer] Failed to analyze file ${filePath}:`, error);
      return this.createMinimalAnalysis(filePath, 'Analysis failed');
    }
  }

  private initializeLanguagePatterns(): void {
    // TypeScript/JavaScript
    this.languagePatterns.set('typescript', {
      extensions: ['.ts', '.tsx'],
      importPatterns: [
        /^import\s+.*from\s+['"`]/m,
        /^import\s*\(/m,
        /require\s*\(['"`]/
      ],
      exportPatterns: [
        /^export\s+(default\s+)?/m,
        /^module\.exports\s*=/m,
        /exports\.\w+\s*=/
      ],
      commentPatterns: [/\/\*[\s\S]*?\*\/|\/\/.*$/gm],
      complexityIndicators: [
        /\bclass\s+\w+/g,
        /\binterface\s+\w+/g,
        /\btype\s+\w+/g,
        /\bfunction\s+\w+/g,
        /\basync\s+function/g,
        /=>\s*{/g,
        /\bif\s*\(/g,
        /\bfor\s*\(/g,
        /\bwhile\s*\(/g,
        /\btry\s*{/g
      ],
      testPatterns: [
        /\b(describe|it|test|expect)\s*\(/g,
        /\.test\.|\.spec\./,
        /__tests__/
      ]
    });

    this.languagePatterns.set('javascript', {
      extensions: ['.js', '.jsx', '.mjs'],
      importPatterns: [
        /^import\s+.*from\s+['"`]/m,
        /^import\s*\(/m,
        /require\s*\(['"`]/
      ],
      exportPatterns: [
        /^export\s+(default\s+)?/m,
        /^module\.exports\s*=/m,
        /exports\.\w+\s*=/
      ],
      commentPatterns: [/\/\*[\s\S]*?\*\/|\/\/.*$/gm],
      complexityIndicators: [
        /\bclass\s+\w+/g,
        /\bfunction\s+\w+/g,
        /\basync\s+function/g,
        /=>\s*{/g,
        /\bif\s*\(/g,
        /\bfor\s*\(/g,
        /\bwhile\s*\(/g,
        /\btry\s*{/g
      ],
      testPatterns: [
        /\b(describe|it|test|expect)\s*\(/g,
        /\.test\.|\.spec\./,
        /__tests__/
      ]
    });

    // Python
    this.languagePatterns.set('python', {
      extensions: ['.py', '.pyx'],
      importPatterns: [
        /^import\s+\w+/m,
        /^from\s+\w+\s+import/m
      ],
      exportPatterns: [
        /^def\s+\w+/m,
        /^class\s+\w+/m,
        /__all__\s*=/
      ],
      commentPatterns: [/#.*$/gm, /"""[\s\S]*?"""/g, /'''[\s\S]*?'''/g],
      complexityIndicators: [
        /\bclass\s+\w+/g,
        /\bdef\s+\w+/g,
        /\basync\s+def/g,
        /\bif\s+/g,
        /\bfor\s+/g,
        /\bwhile\s+/g,
        /\btry:/g,
        /\bwith\s+/g
      ],
      testPatterns: [
        /\btest_\w+/g,
        /\bassert\s+/g,
        /unittest\./,
        /pytest\./
      ]
    });

    // Markdown
    this.languagePatterns.set('markdown', {
      extensions: ['.md', '.markdown'],
      importPatterns: [],
      exportPatterns: [],
      commentPatterns: [/<!--[\s\S]*?-->/g],
      complexityIndicators: [
        /^#{1,6}\s+/gm, // Headers
        /```[\s\S]*?```/g, // Code blocks
        /\[.*?\]\(.*?\)/g, // Links
        /!\[.*?\]\(.*?\)/g // Images
      ],
      testPatterns: []
    });

    // JSON/Config files
    this.languagePatterns.set('json', {
      extensions: ['.json', '.jsonc'],
      importPatterns: [],
      exportPatterns: [],
      commentPatterns: [/\/\*[\s\S]*?\*\/|\/\/.*$/gm],
      complexityIndicators: [
        /"scripts":/,
        /"dependencies":/,
        /"devDependencies":/
      ],
      testPatterns: []
    });

    // YAML
    this.languagePatterns.set('yaml', {
      extensions: ['.yml', '.yaml'],
      importPatterns: [],
      exportPatterns: [],
      commentPatterns: [/#.*$/gm],
      complexityIndicators: [
        /^[a-zA-Z_][\w-]*:/gm, // Top-level keys
        /^\s+-\s/gm // List items
      ],
      testPatterns: []
    });
  }

  private detectLanguage(filePath: string, content: string): string {
    const ext = path.extname(filePath).toLowerCase();
    
    // Check by extension first
    for (const [language, pattern] of this.languagePatterns.entries()) {
      if (pattern.extensions.includes(ext)) {
        return language;
      }
    }

    // Fallback: detect by content patterns
    if (/^import\s+.*from\s+['"`]|interface\s+\w+|type\s+\w+/.test(content)) {
      return 'typescript';
    }
    if (/^import\s+.*from\s+['"`]|require\s*\(['"`]/.test(content)) {
      return 'javascript';
    }
    if (/^import\s+\w+|^from\s+\w+\s+import|^def\s+\w+/.test(content)) {
      return 'python';
    }
    if (/^#{1,6}\s+|\[.*?\]\(.*?\)/.test(content)) {
      return 'markdown';
    }

    return 'unknown';
  }

  private calculateComplexity(content: string, language: string): number {
    const pattern = this.languagePatterns.get(language);
    if (!pattern) return 0;

    let complexity = 0;
    
    // Count complexity indicators
    for (const indicator of pattern.complexityIndicators) {
      const matches = content.match(indicator);
      if (matches) {
        complexity += matches.length;
      }
    }

    // Add line-based complexity
    const lines = content.split('\n').length;
    complexity += Math.floor(lines / 50); // 1 point per 50 lines

    // Add nesting complexity (rough estimate)
    const openBraces = (content.match(/{/g) || []).length;
    const closeBraces = (content.match(/}/g) || []).length;
    complexity += Math.min(openBraces, closeBraces) * 0.5;

    return Math.round(complexity);
  }

  private hasImports(content: string, language: string): boolean {
    const pattern = this.languagePatterns.get(language);
    if (!pattern) return false;

    return pattern.importPatterns.some(regex => regex.test(content));
  }

  private hasExports(content: string, language: string): boolean {
    const pattern = this.languagePatterns.get(language);
    if (!pattern) return false;

    return pattern.exportPatterns.some(regex => regex.test(content));
  }

  private hasTests(content: string, language: string): boolean {
    const pattern = this.languagePatterns.get(language);
    if (!pattern) return false;

    return pattern.testPatterns.some(regex => regex.test(content));
  }

  private hasDocumentation(content: string): boolean {
    // Check for various documentation indicators
    const docIndicators = [
      /\/\*\*[\s\S]*?\*\//g, // JSDoc
      /"""[\s\S]*?"""/g,     // Python docstrings
      /'''[\s\S]*?'''/g,     // Python docstrings
      /^#{1,6}\s+/m,         // Markdown headers
      /@param\s+/g,          // Parameter docs
      /@returns?\s+/g,       // Return docs
      /@description\s+/g,    // Description docs
      /README|CHANGELOG|CONTRIBUTING/i // Doc files
    ];

    return docIndicators.some(regex => regex.test(content));
  }

  private calculateCodeCommentRatio(content: string, language: string): number {
    const pattern = this.languagePatterns.get(language);
    if (!pattern) return 0;

    const lines = content.split('\n');
    let codeLines = 0;
    let commentLines = 0;

    for (const line of lines) {
      const trimmed = line.trim();
      if (trimmed === '') continue;

      let isComment = false;
      for (const commentPattern of pattern.commentPatterns) {
        if (commentPattern.test(trimmed)) {
          isComment = true;
          break;
        }
      }

      if (isComment) {
        commentLines++;
      } else {
        codeLines++;
      }
    }

    return codeLines > 0 ? commentLines / codeLines : 0;
  }

  private countUniqueTokens(content: string): number {
    // Simple tokenization - split by common delimiters
    const tokens = content
      .toLowerCase()
      .replace(/[^\w\s]/g, ' ')
      .split(/\s+/)
      .filter(token => token.length > 2); // Ignore very short tokens

    const uniqueTokens = new Set(tokens);
    return uniqueTokens.size;
  }

  private determineFileType(filePath: string, content: string): ContentAnalysis['fileType'] {
    const fileName = path.basename(filePath).toLowerCase();
    const dirName = path.dirname(filePath).toLowerCase();

    // Test files
    if (fileName.includes('.test.') || fileName.includes('.spec.') || 
        dirName.includes('test') || dirName.includes('spec')) {
      return 'test';
    }

    // Documentation
    if (fileName.includes('readme') || fileName.includes('changelog') ||
        fileName.includes('contributing') || fileName.includes('license') ||
        path.extname(filePath) === '.md') {
      return 'documentation';
    }

    // Configuration files
    if (fileName.includes('config') || fileName.includes('.json') ||
        fileName.includes('.yml') || fileName.includes('.yaml') ||
        fileName.includes('package.json') || fileName.includes('tsconfig') ||
        fileName.includes('.env') || fileName.includes('dockerfile')) {
      return 'config';
    }

    // Build files
    if (fileName.includes('webpack') || fileName.includes('rollup') ||
        fileName.includes('vite') || fileName.includes('build') ||
        fileName.includes('gulpfile') || fileName.includes('makefile')) {
      return 'build';
    }

    // Data files
    if (fileName.includes('.csv') || fileName.includes('.xml') ||
        fileName.includes('.sql') || fileName.includes('data')) {
      return 'data';
    }

    // Default to source code
    return 'source';
  }

  private assessSemanticValue(analysis: ContentAnalysis, content: string): 'high' | 'medium' | 'low' {
    let score = 0;

    // High value indicators
    if (analysis.hasExports) score += 3;
    if (analysis.hasImports) score += 2;
    if (analysis.hasDocumentation) score += 2;
    if (analysis.complexity > 5 && analysis.complexity < 100) score += 2;
    if (analysis.uniqueTokens > 50) score += 1;
    if (analysis.codeToCommentRatio > 0.1 && analysis.codeToCommentRatio < 2) score += 1;

    // File type bonuses
    const typeBonus = {
      'source': 3,
      'test': 2,
      'config': 1,
      'documentation': 2,
      'build': 1,
      'data': 0
    };
    score += typeBonus[analysis.fileType];

    // Language bonuses
    const languageBonus: Record<string, number> = {
      'typescript': 3,
      'javascript': 2,
      'python': 2,
      'markdown': 1,
      'json': 1,
      'yaml': 1,
      'unknown': 0
    };
    score += languageBonus[analysis.language] || 0;

    // Content quality indicators
    if (/TODO|FIXME|HACK|XXX/.test(content)) score += 1;
    if (/class\s+\w+|interface\s+\w+|function\s+\w+/.test(content)) score += 2;
    if (content.includes('export default') || content.includes('module.exports')) score += 2;

    // Penalties for low-value content
    if (content.length < 100) score -= 2;
    if (/console\.log\s*\(\s*['"`]/.test(content)) score -= 1; // Debug logs
    if (/^\s*\/\/.*test.*file/i.test(content)) score -= 1; // Test comment files

    // Convert score to semantic value
    if (score >= 8) return 'high';
    if (score >= 4) return 'medium';
    return 'low';
  }

  private calculateImportance(analysis: ContentAnalysis, filePath: string): number {
    let importance = 0;

    // Base score by semantic value
    const semanticScore = {
      'high': 40,
      'medium': 25,
      'low': 10
    };
    importance += semanticScore[analysis.semanticValue];

    // File type importance
    const typeImportance = {
      'source': 30,
      'test': 20,
      'documentation': 15,
      'config': 10,
      'build': 5,
      'data': 5
    };
    importance += typeImportance[analysis.fileType];

    // Language importance
    const languageImportance: Record<string, number> = {
      'typescript': 20,
      'javascript': 18,
      'python': 15,
      'markdown': 10,
      'json': 8,
      'yaml': 5,
      'unknown': 0
    };
    importance += languageImportance[analysis.language] || 0;

    // Structure bonuses
    if (analysis.hasExports) importance += 10;
    if (analysis.hasImports) importance += 5;
    if (analysis.hasDocumentation) importance += 5;
    if (analysis.hasTests) importance += 8;

    // Complexity bonus (sweet spot)
    if (analysis.complexity > 3 && analysis.complexity < 50) {
      importance += Math.min(analysis.complexity * 0.5, 10);
    }

    // File name importance
    const fileName = path.basename(filePath).toLowerCase();
    if (fileName.includes('index') || fileName.includes('main') || fileName.includes('app')) {
      importance += 15;
    }
    if (fileName.includes('util') || fileName.includes('helper') || fileName.includes('common')) {
      importance += 10;
    }

    return Math.min(Math.round(importance), 100);
  }

  private createMinimalAnalysis(filePath: string, reason: string): ContentAnalysis {
    return {
      language: 'unknown',
      complexity: 0,
      hasImports: false,
      hasExports: false,
      hasTests: false,
      hasDocumentation: false,
      codeToCommentRatio: 0,
      uniqueTokens: 0,
      semanticValue: 'low',
      fileType: 'data',
      estimatedImportance: 0
    };
  }

  // Utility methods for batch analysis
  async analyzeFiles(filePaths: string[]): Promise<Map<string, ContentAnalysis>> {
    const results = new Map<string, ContentAnalysis>();
    
    // Process files in parallel batches to avoid overwhelming the system
    const batchSize = 10;
    for (let i = 0; i < filePaths.length; i += batchSize) {
      const batch = filePaths.slice(i, i + batchSize);
      const batchPromises = batch.map(async (filePath) => {
        const analysis = await this.analyzeFile(filePath);
        return [filePath, analysis] as [string, ContentAnalysis];
      });
      
      const batchResults = await Promise.all(batchPromises);
      for (const [filePath, analysis] of batchResults) {
        results.set(filePath, analysis);
      }
      
      // Small delay between batches to prevent overwhelming the system
      if (i + batchSize < filePaths.length) {
        await new Promise(resolve => setTimeout(resolve, 10));
      }
    }
    
    return results;
  }

  getLanguageStats(analyses: Map<string, ContentAnalysis>): Record<string, number> {
    const stats: Record<string, number> = {};
    
    for (const analysis of analyses.values()) {
      stats[analysis.language] = (stats[analysis.language] || 0) + 1;
    }
    
    return stats;
  }

  getSemanticValueDistribution(analyses: Map<string, ContentAnalysis>): Record<string, number> {
    const distribution = { high: 0, medium: 0, low: 0 };
    
    for (const analysis of analyses.values()) {
      distribution[analysis.semanticValue]++;
    }
    
    return distribution;
  }
}