# Advanced Smart File Filtering
## Exploiting Intelligence for Optimal Code Indexing

### 🧠 **Enhanced Smart Approach**

Let's make the file filtering system truly intelligent by leveraging multiple data sources and ML-like heuristics.

---

## 🔍 **Multi-Dimensional Intelligence**

### **1. Content-Based Intelligence**
```typescript
interface ContentAnalysis {
  language: string;
  complexity: number;
  hasImports: boolean;
  hasExports: boolean;
  hasTests: boolean;
  hasDocumentation: boolean;
  codeToCommentRatio: number;
  uniqueTokens: number;
  semanticValue: 'high' | 'medium' | 'low';
}

class ContentAnalyzer {
  async analyzeFile(filePath: string): Promise<ContentAnalysis> {
    const content = await fs.readFile(filePath, 'utf-8');
    
    return {
      language: this.detectLanguage(filePath, content),
      complexity: this.calculateComplexity(content),
      hasImports: /^(import|require|#include)/m.test(content),
      hasExports: /^(export|module\.exports)/m.test(content),
      hasTests: /\b(test|spec|describe|it)\b/i.test(content),
      hasDocumentation: this.hasDocumentation(content),
      codeToCommentRatio: this.calculateCodeCommentRatio(content),
      uniqueTokens: this.countUniqueTokens(content),
      semanticValue: this.assessSemanticValue(content)
    };
  }
  
  private assessSemanticValue(content: string): 'high' | 'medium' | 'low' {
    // High value indicators
    if (
      /class\s+\w+|function\s+\w+|interface\s+\w+|type\s+\w+/.test(content) ||
      /export\s+(default\s+)?[a-zA-Z]/.test(content) ||
      content.includes('TODO') || content.includes('FIXME')
    ) {
      return 'high';
    }
    
    // Low value indicators  
    if (
      content.length < 100 ||
      /^\/\/.*test.*file/i.test(content) ||
      /console\.log\s*\(\s*['"`]/.test(content) // Debug logs
    ) {
      return 'low';
    }
    
    return 'medium';
  }
}
```

### **2. Behavioral Intelligence**
```typescript
interface FileUsagePattern {
  accessFrequency: number;
  lastAccessed: Date;
  modificationFrequency: number;
  searchHitCount: number;
  referencedByFiles: string[];
  isHotFile: boolean;
  userInteractionScore: number;
}

class BehavioralAnalyzer {
  private fileStats: Map<string, FileUsagePattern> = new Map();
  
  recordFileAccess(filePath: string): void {
    const stats = this.getOrCreateStats(filePath);
    stats.accessFrequency++;
    stats.lastAccessed = new Date();
    stats.userInteractionScore += 1;
  }
  
  recordSearchHit(filePath: string, query: string): void {
    const stats = this.getOrCreateStats(filePath);
    stats.searchHitCount++;
    stats.userInteractionScore += 2; // Search hits are more valuable
  }
  
  recordFileReference(fromFile: string, toFile: string): void {
    const stats = this.getOrCreateStats(toFile);
    if (!stats.referencedByFiles.includes(fromFile)) {
      stats.referencedByFiles.push(fromFile);
      stats.userInteractionScore += 0.5;
    }
  }
  
  isHotFile(filePath: string): boolean {
    const stats = this.fileStats.get(filePath);
    if (!stats) return false;
    
    const recentAccess = Date.now() - stats.lastAccessed.getTime() < 24 * 60 * 60 * 1000; // 24h
    const highFrequency = stats.accessFrequency > 5;
    const wellReferenced = stats.referencedByFiles.length > 2;
    
    return recentAccess && (highFrequency || wellReferenced);
  }
  
  calculatePriority(filePath: string): number {
    const stats = this.fileStats.get(filePath);
    if (!stats) return 0;
    
    let score = 0;
    score += Math.min(stats.accessFrequency * 2, 20); // Max 20 points
    score += Math.min(stats.searchHitCount * 5, 25);   // Max 25 points  
    score += Math.min(stats.referencedByFiles.length * 3, 15); // Max 15 points
    
    // Recency bonus
    const daysSinceAccess = (Date.now() - stats.lastAccessed.getTime()) / (24 * 60 * 60 * 1000);
    if (daysSinceAccess < 1) score += 10;
    else if (daysSinceAccess < 7) score += 5;
    
    return Math.min(score, 100); // Max 100 points
  }
}
```

### **3. Project Context Intelligence**
```typescript
interface ProjectContext {
  projectType: 'library' | 'application' | 'tool' | 'documentation';
  framework: string[];
  buildSystem: string;
  hasTests: boolean;
  hasDocumentation: boolean;
  teamSize: 'solo' | 'small' | 'large';
  developmentPhase: 'prototype' | 'development' | 'maintenance';
}

class ProjectContextAnalyzer {
  async analyzeProject(rootPath: string): Promise<ProjectContext> {
    const packageJson = await this.readPackageJson(rootPath);
    const fileStructure = await this.analyzeFileStructure(rootPath);
    
    return {
      projectType: this.detectProjectType(packageJson, fileStructure),
      framework: this.detectFrameworks(packageJson),
      buildSystem: this.detectBuildSystem(fileStructure),
      hasTests: fileStructure.testFiles > 0,
      hasDocumentation: fileStructure.docFiles > 0,
      teamSize: this.estimateTeamSize(fileStructure),
      developmentPhase: this.detectDevelopmentPhase(fileStructure)
    };
  }
  
  private detectProjectType(packageJson: any, structure: any): ProjectContext['projectType'] {
    if (packageJson?.main || packageJson?.exports) return 'library';
    if (structure.configFiles > 5) return 'application';
    if (packageJson?.bin) return 'tool';
    if (structure.docFiles > structure.codeFiles) return 'documentation';
    return 'application';
  }
  
  getSmartFilters(context: ProjectContext): SmartFilterRules {
    const rules: SmartFilterRules = {
      priorityPatterns: [],
      deprioritizePatterns: [],
      contextualRules: []
    };
    
    // Library projects: prioritize exports and API files
    if (context.projectType === 'library') {
      rules.priorityPatterns.push('**/index.{ts,js}', '**/lib/**', '**/src/**');
      rules.deprioritizePatterns.push('**/examples/**', '**/demo/**');
    }
    
    // Application projects: prioritize main app files
    if (context.projectType === 'application') {
      rules.priorityPatterns.push('**/src/**', '**/app/**', '**/components/**');
      rules.deprioritizePatterns.push('**/vendor/**', '**/third-party/**');
    }
    
    // Test-heavy projects: include test files
    if (context.hasTests) {
      rules.priorityPatterns.push('**/*.test.{ts,js}', '**/*.spec.{ts,js}');
    }
    
    return rules;
  }
}
```

---

## 🎯 **Advanced Filtering Engine**

### **4. ML-Inspired Scoring System**
```typescript
interface FileScore {
  contentScore: number;      // 0-100 based on content analysis
  behavioralScore: number;   // 0-100 based on usage patterns  
  contextScore: number;      // 0-100 based on project context
  gitScore: number;          // 0-100 based on git status
  finalScore: number;        // Weighted combination
  shouldIndex: boolean;
  priority: 'critical' | 'high' | 'medium' | 'low' | 'skip';
}

class AdvancedFileScorer {
  constructor(
    private contentAnalyzer: ContentAnalyzer,
    private behavioralAnalyzer: BehavioralAnalyzer,
    private projectContext: ProjectContext
  ) {}
  
  async scoreFile(filePath: string): Promise<FileScore> {
    // Parallel analysis for performance
    const [contentAnalysis, isGitTracked, behavioralPriority] = await Promise.all([
      this.contentAnalyzer.analyzeFile(filePath),
      this.isGitTracked(filePath),
      Promise.resolve(this.behavioralAnalyzer.calculatePriority(filePath))
    ]);
    
    const contentScore = this.calculateContentScore(contentAnalysis);
    const gitScore = isGitTracked ? 80 : 20; // Strong bias toward git files
    const contextScore = this.calculateContextScore(filePath, contentAnalysis);
    
    // Weighted scoring (tunable)
    const weights = {
      content: 0.3,
      behavioral: 0.25,
      context: 0.25,
      git: 0.2
    };
    
    const finalScore = 
      contentScore * weights.content +
      behavioralPriority * weights.behavioral +
      contextScore * weights.context +
      gitScore * weights.git;
    
    return {
      contentScore,
      behavioralScore: behavioralPriority,
      contextScore,
      gitScore,
      finalScore,
      shouldIndex: finalScore >= 40, // Threshold
      priority: this.scoreToPriority(finalScore)
    };
  }
  
  private calculateContentScore(analysis: ContentAnalysis): number {
    let score = 0;
    
    // Language bonus
    const languageBonus = {
      'typescript': 20,
      'javascript': 18,
      'python': 15,
      'markdown': 10,
      'json': 8
    };
    score += languageBonus[analysis.language] || 5;
    
    // Semantic value
    const semanticBonus = {
      'high': 30,
      'medium': 15,
      'low': 5
    };
    score += semanticBonus[analysis.semanticValue];
    
    // Structure bonuses
    if (analysis.hasImports) score += 10;
    if (analysis.hasExports) score += 15;
    if (analysis.hasTests) score += 12;
    if (analysis.hasDocumentation) score += 8;
    
    // Complexity bonus (but not too complex)
    if (analysis.complexity > 5 && analysis.complexity < 50) {
      score += Math.min(analysis.complexity, 20);
    }
    
    return Math.min(score, 100);
  }
  
  private scoreToPriority(score: number): FileScore['priority'] {
    if (score >= 80) return 'critical';
    if (score >= 65) return 'high';
    if (score >= 40) return 'medium';
    if (score >= 20) return 'low';
    return 'skip';
  }
}
```

### **5. Adaptive Learning System**
```typescript
class AdaptiveLearningSystem {
  private scoringHistory: Map<string, FileScore[]> = new Map();
  private userFeedback: Map<string, 'useful' | 'not-useful'> = new Map();
  
  recordScoring(filePath: string, score: FileScore): void {
    if (!this.scoringHistory.has(filePath)) {
      this.scoringHistory.set(filePath, []);
    }
    this.scoringHistory.get(filePath)!.push(score);
  }
  
  recordUserFeedback(filePath: string, feedback: 'useful' | 'not-useful'): void {
    this.userFeedback.set(filePath, feedback);
  }
  
  // Learn from search patterns
  recordSearchResult(query: string, filePath: string, wasClicked: boolean): void {
    if (wasClicked) {
      this.behavioralAnalyzer.recordSearchHit(filePath, query);
    }
  }
  
  // Adjust scoring weights based on feedback
  getAdaptiveWeights(): ScoringWeights {
    const baseWeights = { content: 0.3, behavioral: 0.25, context: 0.25, git: 0.2 };
    
    // Analyze feedback patterns
    let gitFilesUseful = 0;
    let nonGitFilesUseful = 0;
    let totalGitFiles = 0;
    let totalNonGitFiles = 0;
    
    for (const [filePath, feedback] of this.userFeedback.entries()) {
      const isGit = this.isGitTracked(filePath);
      
      if (isGit) {
        totalGitFiles++;
        if (feedback === 'useful') gitFilesUseful++;
      } else {
        totalNonGitFiles++;
        if (feedback === 'useful') nonGitFilesUseful++;
      }
    }
    
    // Adjust git weight based on usefulness ratio
    if (totalGitFiles > 10 && totalNonGitFiles > 10) {
      const gitUsefulnessRatio = gitFilesUseful / totalGitFiles;
      const nonGitUsefulnessRatio = nonGitFilesUseful / totalNonGitFiles;
      
      if (nonGitUsefulnessRatio > gitUsefulnessRatio) {
        // Non-git files are more useful, reduce git bias
        baseWeights.git *= 0.8;
        baseWeights.content *= 1.1;
      }
    }
    
    return baseWeights;
  }
}
```

---

## 🚀 **Advanced Features**

### **6. Predictive Indexing**
```typescript
class PredictiveIndexer {
  private fileRelationships: Map<string, string[]> = new Map();
  private editPatterns: Map<string, Date[]> = new Map();
  
  // Predict which files might be edited next
  predictNextFiles(currentFile: string): string[] {
    const related = this.fileRelationships.get(currentFile) || [];
    const recentlyEdited = this.getRecentlyEditedFiles();
    
    // Combine relationship and temporal patterns
    const predictions = new Set([...related, ...recentlyEdited]);
    
    return Array.from(predictions)
      .filter(f => f !== currentFile)
      .slice(0, 10); // Top 10 predictions
  }
  
  // Pre-index predicted files
  async preIndexFiles(predictions: string[]): Promise<void> {
    for (const filePath of predictions) {
      const score = await this.scorer.scoreFile(filePath);
      if (score.shouldIndex && score.priority !== 'skip') {
        // Queue for background indexing
        this.embeddingQueue.enqueue({
          filePath,
          priority: 'background',
          reason: 'predictive'
        });
      }
    }
  }
}
```

### **7. Team Collaboration Intelligence**
```typescript
class TeamIntelligence {
  // Learn from team patterns
  async analyzeTeamPatterns(): Promise<TeamPatterns> {
    const gitLog = await this.git.log(['--since=30 days ago', '--name-only']);
    
    const patterns = {
      hotFiles: this.findHotFiles(gitLog),
      collaborativeFiles: this.findCollaborativeFiles(gitLog),
      expertiseAreas: this.mapExpertiseAreas(gitLog),
      workflowPatterns: this.detectWorkflowPatterns(gitLog)
    };
    
    return patterns;
  }
  
  // Boost files that team frequently collaborates on
  getTeamBoost(filePath: string): number {
    const teamPatterns = this.getTeamPatterns();
    
    let boost = 0;
    if (teamPatterns.hotFiles.includes(filePath)) boost += 20;
    if (teamPatterns.collaborativeFiles.includes(filePath)) boost += 15;
    
    return boost;
  }
}
```

---

## 🎛️ **Configuration & Control**

### **8. Dynamic Configuration**
```typescript
interface SmartFilterConfig {
  // Scoring weights (sum to 1.0)
  weights: {
    content: number;
    behavioral: number;
    context: number;
    git: number;
    team: number;
  };
  
  // Thresholds
  thresholds: {
    indexingThreshold: number;    // Minimum score to index
    priorityThresholds: {
      critical: number;
      high: number;
      medium: number;
      low: number;
    };
  };
  
  // Learning settings
  learning: {
    enabled: boolean;
    adaptWeights: boolean;
    feedbackWeight: number;
    historyWindow: number; // days
  };
  
  // Predictive features
  predictive: {
    enabled: boolean;
    preIndexCount: number;
    relationshipDepth: number;
  };
}
```

### **9. MCP Tools for Control**
```typescript
const advancedFilteringTools = [
  {
    name: "analyze_file_score",
    description: "Get detailed scoring breakdown for a file",
    inputSchema: {
      type: "object",
      properties: {
        filePath: { type: "string" }
      }
    }
  },
  
  {
    name: "tune_filtering_weights", 
    description: "Adjust scoring weights for file filtering",
    inputSchema: {
      type: "object",
      properties: {
        weights: {
          type: "object",
          properties: {
            content: { type: "number", minimum: 0, maximum: 1 },
            behavioral: { type: "number", minimum: 0, maximum: 1 },
            context: { type: "number", minimum: 0, maximum: 1 },
            git: { type: "number", minimum: 0, maximum: 1 }
          }
        }
      }
    }
  },
  
  {
    name: "get_filtering_insights",
    description: "Get insights about current filtering performance",
    inputSchema: { type: "object", properties: {} }
  }
];
```

---

This advanced smart approach transforms file filtering from simple pattern matching into an intelligent, adaptive system that learns from usage patterns, understands project context, and continuously improves its decisions.

**Key Benefits:**
- 🧠 **Learns from behavior** - Files you actually use get prioritized
- 🎯 **Context-aware** - Different strategies for different project types  
- 📈 **Adaptive** - Improves over time based on feedback
- 🔮 **Predictive** - Pre-indexes files you're likely to need
- 👥 **Team-aware** - Learns from collaborative patterns

Would you like me to implement any of these advanced components?