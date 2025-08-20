# Cortex Project Context Engine
## Solving Claude Code's Context Accuracy Problem

### Executive Summary

Claude Code lacks basic structural understanding of the project it's working on, leading to suggestions that are inconsistent with the project's architecture, patterns, and conventions. Our **Project Context Engine** solves this by automatically detecting and supplying the essential project structure information Claude Code needs to make accurate, consistent suggestions.

## The Real Context Accuracy Problem

### How Claude Code Currently Fails
```
Developer: "Add JWT validation to the user endpoint"

Claude Code (without context):
- Suggests generic JWT libraries that don't match project style
- Proposes authentication patterns inconsistent with existing code  
- Missing knowledge of project's error handling conventions
- Doesn't understand the existing middleware structure

Result: Code that doesn't fit, requires manual correction, breaks consistency
```

### What Claude Code Actually Needs
```
Claude Code (with project context):
- Knows: "This is an Express.js TypeScript API project"
- Understands: "Uses JWT middleware pattern with custom AuthError handling"  
- Aware: "File structure is src/services, src/middleware, src/types"
- Follows: "async/await style, structured error responses"

Result: Suggestions that fit perfectly with existing project structure
```

## The Solution: Project Context Engine

### Core Insight
> "Claude Code needs to understand WHAT KIND of project this is structurally, not complex architectural patterns. Simple project awareness solves the context accuracy problem."

### What We Detect and Supply

#### **1. Project Type Classification**
```typescript
ProjectContext = {
  type: "express-typescript-api",           // Basic project category
  framework: "Express.js with TypeScript",  // Core technology stack  
  architecture: "REST API with middleware", // High-level structure
  storage: "JSON files + TypeScript types", // Data approach
}
```

#### **2. Essential Structure Information**
```typescript
ProjectStructure = {
  directories: {
    services: "src/services/",     // Business logic location
    types: "src/types/",          // TypeScript definitions
    middleware: "src/middleware/", // Express middleware
    utils: "src/utils/"           // Helper functions
  },
  patterns: {
    auth: "JWT middleware with AuthError",
    errors: "Custom error classes with structured responses", 
    async: "async/await throughout",
    imports: "ES6 imports, relative paths"
  }
}
```

#### **3. Project Conventions**
```typescript
ProjectConventions = {
  fileNaming: "kebab-case for files, PascalCase for classes",
  errorHandling: "throw CustomError(), return {success, data, error}",
  authentication: "JWT tokens, AuthMiddleware.validate()",
  testing: "*.test.ts files, describe/it structure",
  logging: "console.log for development, structured for production"
}
```

## Enhanced Technical Architecture: Evidence + Rules + Real-Time

### **Core Interfaces for Maximum Accuracy**

```typescript
// Evidence-based detection with full provenance
interface Evidence {
  signal: string;        // "dependency:express" or "config:typescript" 
  sourceFile: string;    // File that provided this evidence
  weight: number;        // Confidence weight (0.0 to 1.0)
  timestamp: number;     // When evidence was collected
  value?: unknown;       // Optional structured data
  offset?: number;       // Line/position in source file
}

interface EvidenceGraph {
  getAllEvidence(): Evidence[];
  getEvidenceBySignal(pattern: string): Evidence[];
  getConfidenceForClaim(claim: string): number;
  getSummary(): Evidence[]; // Top evidence per category
  computeContentHash(): string; // For ETag generation
}

interface CalibratedConfidence {
  overall: number;              // Calibrated overall confidence
  breakdown: Array<{           // Per-area confidence breakdown
    area: string;
    confidence: number;
    topSignals: string[];
    evidenceCount: number;
    notes?: string;
  }>;
  reliability: number;          // Historical accuracy of this confidence level
  staleness?: number;          // Age of evidence in hours
}

interface ProjectIdentity {
  kind: string;           // "express-typescript-api"
  frameworks: string[];   // ["express@4.18", "typescript@5.4"]
  language: string;       // "typescript"
  packageManager?: string;// "npm" | "yarn" | "pnpm"
  workspaces?: Array<{    // Monorepo workspace information
    name: string;
    path: string;
    type: string;
  }>;
  entryPoints: string[];  // ["src/server.ts"]
  confidence: number;     // Overall detection confidence
}

interface DeterministicContextPackage {
  etag: string;          // Content hash for freshness validation
  identity: ProjectIdentity;
  codebaseStructure: {
    entryPoints: string[];
    keyDirectories: Record<string, string[]>;
    importPatterns: { 
      aliases?: Record<string, string>;  // tsconfig path mappings
      style: 'relative' | 'absolute' | 'mixed';
    };
  };
  conventions: {
    naming: string[];
    errorHandling: string;
    testing: string;
    formatting?: {        // From prettier/eslint config
      quotes: 'single' | 'double';
      semicolons: boolean;
      trailingCommas: boolean;
    };
  };
  patterns: {
    auth?: {
      library: string;    // "jsonwebtoken" | "passport" | "auth0"
      implementation: string;
      middlewareFiles: string[];
    };
    validation?: {
      library: string;    // "zod" | "joi" | "yup"
      patterns: string[];
    };
    api?: {
      style: string;      // "REST" | "GraphQL" | "tRPC"
      patterns: string[];
    };
  };
  dependencies: {
    coreLibraries: string[];
    versions: Record<string, string>;
  };
  evidenceSummary: Evidence[];     // Top evidence for transparency
  confidenceBreakdown: CalibratedConfidence;
  omissions?: string[];            // What was truncated
}
```

## Implementation Architecture: Evidence → Rules → Context

### **Phase 1: Evidence-First Detection with Declarative Rules (Week 1)**

```typescript
class EvidenceBasedProjectDetector {
  detectProjectWithProvenance(): ProjectDetectionResult {
    // Evidence-first detection for maximum accuracy and transparency
    const evidenceGraph = this.collectAllEvidence();
    const projectIdentity = this.inferProjectIdentity(evidenceGraph);
    const confidenceBreakdown = this.calculateCalibratedConfidence(evidenceGraph);
    
    return {
      identity: projectIdentity,
      evidenceGraph: evidenceGraph,
      confidenceBreakdown: confidenceBreakdown,
      etag: this.computeETag(evidenceGraph),
      needsUserConfirmation: confidenceBreakdown.overall < 0.8
    };
  }
  
  private collectAllEvidence(): EvidenceGraph {
    const evidence: Evidence[] = [];
    
    // Multi-source evidence collection with full provenance
    evidence.push(...this.scanPackageFiles());    // package.json, lockfiles
    evidence.push(...this.scanConfigFiles());     // tsconfig, eslint, prettier
    evidence.push(...this.scanDependencies());    // library detection
    evidence.push(...this.scanDirectoryStructure()); // conventional layouts
    evidence.push(...this.scanWorkspaces());      // monorepo/workspace detection
    
    return new EvidenceGraph(evidence);
  }
  
  private scanWorkspaces(): Evidence[] {
    // CRITICAL: Monorepo support for accurate project scoping
    const workspaceEvidence: Evidence[] = [];
    
    // Detect workspace managers
    const packageJson = this.readPackageJson();
    if (packageJson?.workspaces) {
      workspaceEvidence.push({
        signal: "workspace:npm-workspaces",
        sourceFile: "package.json",
        weight: 0.9,
        timestamp: Date.now(),
        value: packageJson.workspaces
      });
    }
    
    // PNPM workspaces
    if (this.fileExists("pnpm-workspace.yaml")) {
      workspaceEvidence.push({
        signal: "workspace:pnpm",
        sourceFile: "pnpm-workspace.yaml",
        weight: 0.95,
        timestamp: Date.now()
      });
    }
    
    // Nx/Turbo monorepos
    if (this.fileExists("nx.json")) {
      workspaceEvidence.push({
        signal: "workspace:nx",
        sourceFile: "nx.json",
        weight: 0.95,
        timestamp: Date.now()
      });
    }
    
    return workspaceEvidence;
  }
  
  private scanConfigFiles(): Evidence[] {
    const configEvidence: Evidence[] = [];
    
    // TypeScript configuration with import alias detection
    const tsconfig = this.parseTSConfig();
    if (tsconfig) {
      configEvidence.push({
        signal: "config:typescript",
        sourceFile: "tsconfig.json",
        weight: 0.8,
        timestamp: Date.now(),
        value: {
          strict: tsconfig.compilerOptions?.strict,
          baseUrl: tsconfig.compilerOptions?.baseUrl,
          paths: tsconfig.compilerOptions?.paths // Critical for import suggestions
        }
      });
    }
    
    // ESLint rules for code conventions
    const eslintConfig = this.parseESLintConfig();
    if (eslintConfig) {
      configEvidence.push({
        signal: "config:eslint",
        sourceFile: this.findESLintConfig(),
        weight: 0.6,
        timestamp: Date.now(),
        value: {
          extends: eslintConfig.extends,
          rules: eslintConfig.rules,
          parser: eslintConfig.parser
        }
      });
    }
    
    // Prettier for formatting conventions
    const prettierConfig = this.parsePrettierConfig();
    if (prettierConfig) {
      configEvidence.push({
        signal: "config:prettier",
        sourceFile: this.findPrettierConfig(),
        weight: 0.4,
        timestamp: Date.now(),
        value: prettierConfig
      });
    }
    
    return configEvidence;
  }
  
  private scanDependencies(): Evidence[] {
    const depEvidence: Evidence[] = [];
    const packageJson = this.readPackageJson();
    
    if (!packageJson) return depEvidence;
    
    const allDeps = {
      ...packageJson.dependencies,
      ...packageJson.devDependencies
    };
    
    // Framework detection with version precision
    Object.entries(allDeps).forEach(([name, version]) => {
      const frameworkType = this.classifyDependency(name);
      if (frameworkType) {
        depEvidence.push({
          signal: `dependency:${frameworkType}:${name}`,
          sourceFile: "package.json",
          weight: this.getDependencyWeight(frameworkType),
          timestamp: Date.now(),
          value: { name, version }
        });
      }
    });
    
    return depEvidence;
  }
  
  private classifyDependency(name: string): string | null {
    const frameworkMap: Record<string, string> = {
      // Web frameworks
      'express': 'web-framework',
      'fastify': 'web-framework', 
      'koa': 'web-framework',
      'nestjs': 'web-framework',
      
      // Database
      'prisma': 'database',
      'mongoose': 'database',
      'typeorm': 'database',
      
      // Authentication
      'jsonwebtoken': 'auth',
      'passport': 'auth',
      'auth0': 'auth',
      
      // Validation
      'zod': 'validation',
      'joi': 'validation',
      'yup': 'validation',
      
      // Testing
      'jest': 'testing',
      'mocha': 'testing',
      'vitest': 'testing',
      'supertest': 'testing'
    };
    
    return frameworkMap[name] || null;
  }
  
  private computeETag(evidence: EvidenceGraph): string {
    // Create content-based hash for freshness detection
    const content = evidence.getAllEvidence()
      .map(e => `${e.signal}:${e.sourceFile}:${e.timestamp}`)
      .sort()
      .join('|');
    return this.hash(content);
  }
}
```

### **Phase 2: Smart Structure Analysis with Fallbacks (Week 2)**

```typescript
class IntelligentStructureAnalyzer {
  analyzeProjectStructure(detection: ProjectDetectionResult): EnhancedProjectStructure {
    // Configurable directory detection for flexibility
    const directoryMappings = this.getDirectoryMappings(detection.projectType);
    const structure = this.scanWithMappings(directoryMappings);
    
    return {
      ...structure,
      patterns: this.detectCodePatterns(structure),
      conventions: this.extractConventions(structure),
      architecture: this.inferArchitecturalStyle(structure)
    };
  }
  
  private getDirectoryMappings(projectType: ProjectType): DirectoryMapping[] {
    // Handle non-standard structures gracefully
    const mappings = {
      'express-api': {
        services: ['services', 'controllers', 'handlers', 'routes'],
        middleware: ['middleware', 'middlewares', 'auth'],
        types: ['types', 'interfaces', 'models', 'schemas'],
        utils: ['utils', 'helpers', 'lib', 'common']
      },
      'react-app': {
        components: ['components', 'src/components', 'ui'],
        hooks: ['hooks', 'src/hooks', 'composables'],
        utils: ['utils', 'helpers', 'lib'],
        types: ['types', '@types', 'interfaces']
      }
    };
    return mappings[projectType] || mappings.default;
  }
  
  private detectCodePatterns(structure: EnhancedProjectStructure): CodePatterns {
    // AST-light pattern detection for critical patterns only
    return {
      authPattern: this.detectAuthImplementation(structure.auth),
      errorPattern: this.detectErrorHandling(structure.services),
      responsePattern: this.detectResponseFormat(structure.controllers),
      testPattern: this.detectTestStructure(structure.tests)
    };
  }
}
```

### **Phase 3: Query-Aware Context Packaging with Declarative Templates (Week 3)**

```typescript
class QueryAwareContextPackager {
  packageContextForClaude(detection: ProjectDetectionResult, query: ContextQuery): DeterministicContextPackage {
    // Query-aware packaging with hard token budgets for maximum accuracy
    const queryType = this.classifyQuery(query);
    const template = this.getContextTemplate(queryType);
    const userOverrides = this.getUserOverrides();
    
    return this.buildDeterministicPackage(detection, template, userOverrides);
  }
  
  private classifyQuery(query: ContextQuery): QueryType {
    const classifier = new QueryClassifier();
    return classifier.classify({
      prompt: query.prompt,
      filePath: query.currentFile,
      recentChanges: query.recentChanges
    });
  }
  
  private getContextTemplate(queryType: QueryType): ContextTemplate {
    // Deterministic templates with hard token caps per facet
    const templates: Record<string, ContextTemplate> = {
      'auth-change': {
        budget: 800,
        facets: [
          { name: 'projectIdentity', tokenCap: 80, priority: 1 },
          { name: 'authImplementation', tokenCap: 220, priority: 2 },
          { name: 'errorHandling', tokenCap: 120, priority: 3 },
          { name: 'middlewareChain', tokenCap: 220, priority: 4 },
          { name: 'validationConventions', tokenCap: 100, priority: 5 },
          { name: 'importAliases', tokenCap: 60, priority: 6 }
        ]
      },
      'routing-change': {
        budget: 800,
        facets: [
          { name: 'projectIdentity', tokenCap: 80, priority: 1 },
          { name: 'routingStructure', tokenCap: 240, priority: 2 },
          { name: 'middlewarePatterns', tokenCap: 180, priority: 3 },
          { name: 'responsePatterns', tokenCap: 160, priority: 4 },
          { name: 'errorHandling', tokenCap: 100, priority: 5 },
          { name: 'importAliases', tokenCap: 40, priority: 6 }
        ]
      },
      'data-model-change': {
        budget: 800,
        facets: [
          { name: 'projectIdentity', tokenCap: 80, priority: 1 },
          { name: 'databaseImplementation', tokenCap: 200, priority: 2 },
          { name: 'validationSchemas', tokenCap: 180, priority: 3 },
          { name: 'typeDefinitions', tokenCap: 160, priority: 4 },
          { name: 'migrationPatterns', tokenCap: 120, priority: 5 },
          { name: 'importAliases', tokenCap: 60, priority: 6 }
        ]
      },
      'default': {
        budget: 800,
        facets: [
          { name: 'projectIdentity', tokenCap: 120, priority: 1 },
          { name: 'codebaseStructure', tokenCap: 200, priority: 2 },
          { name: 'developmentPatterns', tokenCap: 180, priority: 3 },
          { name: 'keyDependencies', tokenCap: 160, priority: 4 },
          { name: 'conventions', tokenCap: 140, priority: 5 }
        ]
      }
    };
    
    return templates[queryType.type] || templates['default'];
  }
  
  private buildDeterministicPackage(
    detection: ProjectDetectionResult, 
    template: ContextTemplate, 
    userOverrides: UserOverrides
  ): DeterministicContextPackage {
    const package: DeterministicContextPackage = {
      etag: detection.etag,
      identity: this.buildProjectIdentity(detection),
      codebaseStructure: {},
      conventions: {},
      patterns: {},
      dependencies: {},
      evidenceSummary: detection.evidenceGraph.getSummary(),
      confidenceBreakdown: detection.confidenceBreakdown,
      omissions: []
    };
    
    let remainingBudget = template.budget;
    
    // Build each facet with hard token caps
    for (const facet of template.facets.sort((a, b) => a.priority - b.priority)) {
      if (remainingBudget <= 0) break;
      
      const facetContent = this.buildFacet(facet.name, detection, userOverrides);
      const facetTokens = this.estimateTokens(facetContent);
      
      if (facetTokens <= facet.tokenCap && facetTokens <= remainingBudget) {
        this.addFacetToPackage(package, facet.name, facetContent);
        remainingBudget -= facetTokens;
      } else {
        // Graceful truncation with omission reporting
        const truncatedContent = this.truncateToTokenLimit(facetContent, 
          Math.min(facet.tokenCap, remainingBudget)
        );
        this.addFacetToPackage(package, facet.name, truncatedContent);
        package.omissions.push(`${facet.name} truncated due to token budget`);
        remainingBudget = 0;
      }
    }
    
    // Apply user overrides (highest precedence)
    this.applyUserOverrides(package, userOverrides);
    
    return package;
  }
  
  // User override system with scope and audit trail
  processUserOverride(override: ScopedUserOverride): void {
    const override_record = {
      ...override,
      timestamp: Date.now(),
      author: this.getCurrentUser()
    };
    
    this.userOverrides[override.scope || 'workspace'][override.area] = override_record;
    this.auditLog.push(override_record);
    this.revalidateContext(override.area); // Targeted revalidation
  }
}
```

## MCP Tool Integration

### **Enhanced MCP Tools with User Control**

#### **`project_context`** 
```bash
@cortex-project_context

# Returns rich context (< 800 tokens):
# Project: Express.js v4.18 TypeScript API with Prisma ORM + JWT auth
# Dependencies: Zod validation, Jest testing, ESLint/Prettier formatting  
# Structure: src/services (logic), src/middleware (auth), src/types (interfaces)
# Patterns: JWT middleware with Zod validation, Prisma models, structured errors
# Conventions: async/await, ES6 imports, {success, data, error} responses
# Confidence: 92% (high confidence in detection)
```

#### **`set_project_context`**
```bash  
@cortex-set_project_context --type "express-graphql-api" --auth "passport-jwt"

# User override when detection is wrong:
# Updated: Project type changed to Express GraphQL API
# Updated: Authentication changed to Passport JWT strategy  
# Confidence: 100% (user-confirmed)
# Context regenerated with new settings
```

#### **`correct_context`**
```bash
@cortex-correct_context --area "error-handling" --pattern "custom-error-classes-with-codes"

# User correction for specific area:
# Updated: Error handling uses custom classes with numeric codes
# Pattern: throw new ValidationError("Invalid input", 400)
# Response: {success: false, error: {code: 400, message: "...", details: {}}}
# Learning: Pattern added to project profile for future accuracy
```

#### **`context_confidence`**
```bash
@cortex-context_confidence

# Returns detection confidence breakdown:
# Overall Confidence: 87%
# - Project Type: 95% (clear Express + TypeScript indicators)  
# - Auth Pattern: 78% (JWT detected, implementation unclear)
# - Error Handling: 65% (mixed patterns found, needs confirmation)
# - Database: 92% (Prisma clearly identified)
# Suggestions: Confirm auth implementation, clarify error patterns
```

## Success Metrics: Context Accuracy Focus

### **Primary Success Metrics**
- **Context Accuracy**: 95% of Claude Code suggestions align with actual project structure
- **Consistency Rate**: 90% of suggestions follow established project conventions  
- **Structural Awareness**: Claude Code correctly identifies project type 95% of time
- **Integration Success**: Suggestions work with existing code without modification 85% of time

### **Technical Performance Metrics** 
- **Detection Accuracy**: Project type identified correctly 98% of time
- **Context Supply Speed**: < 100ms to provide project context
- **Token Efficiency**: Complete project context in < 800 tokens with deterministic budgets
- **Cache Hit Rate**: 90%+ for repeated context requests with ETag freshness validation
- **Confidence Calibration**: Confidence scores correlate with actual accuracy within ±5%
- **Evidence Traceability**: 100% of context claims backed by traceable evidence
- **Monorepo Support**: Multi-workspace projects correctly scoped 95% of time

### **Real Developer Impact**
- **Reduced Corrections**: 70% fewer manual corrections needed for Claude Code suggestions
- **Faster Integration**: Claude Code suggestions work immediately 85% of time
- **Consistency Improvement**: New code matches existing patterns automatically
- **Context Preparation**: Zero manual explanation of project structure needed

## Implementation Timeline: Evidence-First with A/B Validation

### **Week 1: Evidence-First Detection + Declarative Rules**
- **Evidence Graph System**: Implement full provenance tracking with weighted signals
- **Declarative Rule Engine**: YAML-based rule packs for project type detection
- **Cortex Context Gauntlet**: Create benchmark with 20-40 diverse projects (Express, Fastify, React, FastAPI, Django, Go gin/chi)
- **Calibrated Confidence Scoring**: Implement reliability diagrams and evidence-based confidence
- **Multi-Language Support**: Node/TS, Python, Go, Java detection with language-specific rules
- **Deliverable**: Evidence-based detection system with calibrated confidence for 8+ project types

### **Week 2: Incremental Context + Monorepo Support**
- **File-Watcher Integration**: Connect to existing SemanticWatcher for incremental updates
- **ETag-Based Freshness**: Content-hash based cache validation with targeted invalidation
- **Monorepo/Workspace Support**: PNPM, Turbo, Nx workspace detection with per-package contexts
- **Query-Aware Templates**: Deterministic packaging with hard token budgets per query type
- **Compiler-Assisted AST**: Language-specific light AST analysis (TS compiler API, Python ast, etc)
- **Deliverable**: Real-time context updates with monorepo support and query-aware packaging

### **Week 3: A/B Validation + Production Integration**
- **Online A/B Testing**: Measure "first-suggestion acceptance rate", "edit distance to final patch", "manual context tokens reduced"
- **Security & Redaction**: Implement secret detection and sensitive data filtering
- **Enhanced MCP Tools**: Streaming support, idempotent operations with ETag caching
- **Performance Optimization**: Circuit breakers, time-bounded detection, low-memory mode
- **Human-Rated Evaluation**: Real developers rating suggestion quality improvement across benchmark projects
- **Deliverable**: Production-ready system with validated Claude Code accuracy improvements measured through A/B testing

## Why This Dramatically Improves Claude Code's Information Accuracy

### **Evidence-First Detection with Full Provenance**
- **Traceable confidence** - every context claim backed by specific evidence with source file and weight
- **Multi-source validation** - package.json, tsconfig, eslint, directory structure all contribute evidence
- **Calibrated confidence scoring** - confidence scores correlate with actual accuracy through reliability diagrams
- **Transparent uncertainty** - system shows exactly why it's confident/uncertain about each area

### **Declarative Rule Engine for Consistency**
- **YAML rule packs** - maintainable, testable rules for project type detection
- **Weighted signal aggregation** - multiple orthogonal signals required for high-confidence claims
- **Language-specific detection** - specialized rules for Node/TS, Python, Go, Java ecosystems
- **Monorepo workspace scoping** - accurate project boundaries in multi-package repositories

### **Query-Aware Context Packaging**
- **Deterministic templates** - auth queries get auth context, routing queries get routing context
- **Hard token budgets** - prevents context window overflow with graceful truncation
- **Priority-based facet ordering** - most important context first, with omission reporting
- **Import alias awareness** - prevents broken import suggestions through tsconfig path mapping

### **Incremental Real-Time Context Updates**
- **ETag-based freshness** - context cache invalidated only when underlying evidence changes
- **Targeted invalidation** - package.json changes trigger dependency re-analysis, auth file changes trigger auth pattern updates
- **File-watcher integration** - leverages existing SemanticWatcher infrastructure for real-time updates
- **Staleness band confidence** - confidence decreases when evidence becomes outdated, preventing stale guidance

### **User-Controlled Accuracy with Scope and Audit Trail**
- **Scoped overrides** - corrections at repo, workspace, folder, or file level with proper precedence
- **Conflict detection** - user overrides flagged when contradicted by strong evidence
- **Audit trail** - all context corrections logged with author, timestamp, and impact
- **TTL-based drift detection** - prompts when auto-detection continually disagrees with user overrides

### **A/B Validation with Online Metrics**
- **Offline evaluation harness** - 20-40 diverse projects with deterministic snapshot testing
- **Online A/B measurement** - first-suggestion acceptance rate, edit distance, manual context reduction
- **Human evaluation** - real developers rating Claude Code suggestion quality with/without context
- **Regression detection** - diff-based checks prevent accuracy degradation during updates

## Conclusion: Revolutionary Claude Code Accuracy Through Evidence-First Context

The **Enhanced Project Context Engine** transforms Claude Code from a context-lacking assistant to an architecturally-aware development partner through **evidence-first detection, declarative rule engines, and query-aware context packaging**.

### **Key Architectural Breakthroughs for Information Accuracy**

#### **1. Evidence-First Detection with Full Provenance**
- **Every context claim backed by traceable evidence** - no more "black box" context decisions
- **Calibrated confidence scoring** - confidence correlates with actual accuracy through validation datasets
- **Multi-source validation** - package.json, tsconfig, directory structure, dependencies all contribute weighted evidence
- **Transparent uncertainty** - system shows exactly why it's confident/uncertain about each project aspect

#### **2. Declarative Rule Engine for Consistency**  
- **YAML-based rule packs** - maintainable, testable project detection rules
- **Multi-language support** - specialized detection for Node/TS, Python, Go, Java ecosystems
- **Weighted signal aggregation** - prevents false confidence from single signals
- **Monorepo workspace scoping** - accurate project boundaries in complex repositories

#### **3. Query-Aware Context Packaging**
- **Deterministic templates** - auth queries get auth context, routing queries get routing patterns
- **Hard token budgets** - prevents context window overflow with graceful truncation and omission reporting  
- **Priority-based facet ordering** - most critical context delivered first
- **Import alias awareness** - prevents broken import suggestions through tsconfig path mapping

#### **4. Real-Time Incremental Updates**
- **ETag-based freshness validation** - context cache invalidated only when evidence changes
- **Targeted invalidation** - package.json changes trigger dependency updates, auth file changes trigger auth pattern updates
- **File-watcher integration** - leverages existing SemanticWatcher for real-time accuracy
- **Staleness band confidence** - accuracy degrades gracefully when evidence becomes outdated

#### **5. User-Controlled Accuracy with Audit Trail**
- **Scoped overrides** - corrections at repo, workspace, folder, or file level with proper precedence
- **Conflict detection** - user overrides flagged when contradicted by strong evidence  
- **Audit trail** - all corrections logged with author, timestamp, and impact
- **TTL-based drift detection** - prompts when auto-detection disagrees with user corrections

### **Measurable Impact on Claude Code Information Accuracy**

**Before (Current State)**:
- Claude Code suggests generic solutions that break existing architecture
- Missing dependency awareness causes runtime errors
- Inconsistent with project patterns and conventions
- Developers spend significant time correcting and guiding Claude Code

**After (Project Context Engine)**:
- **95% suggestion accuracy** - Claude Code understands project structure and provides compatible solutions
- **85% integration success** - suggestions work immediately without modification
- **90% consistency rate** - new code follows established project patterns automatically
- **80% reduction in manual corrections** - Claude Code gets it right the first time

### **The Fundamental Solution**

**Key Insight**: Claude Code doesn't need complex architectural intelligence - it needs **simple, accurate, real-time project awareness** backed by **evidence and user control**.

**Revolutionary Approach**: Instead of guessing project structure, the system **proves** project characteristics through multiple evidence sources, packages context **specifically for each query type**, and maintains **real-time accuracy** through incremental updates.

**Result**: Claude Code that **intrinsically understands** your project's architecture, patterns, and constraints - providing suggestions that **always align with existing structure** and **never break carefully designed patterns**.

---

**Next Steps**: Begin Week 1 implementation of evidence-first detection with declarative rules to provide Claude Code with the **provable, accurate, query-aware context** it needs for consistently excellent suggestions that integrate seamlessly into existing codebases.