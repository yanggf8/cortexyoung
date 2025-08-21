# Cortex Proactive Context Engine
## Preventing Claude Code Context Issues Through Intelligent CLAUDE.md Maintenance

### Executive Summary

Claude Code's context accuracy problems stem from **lack of project awareness at startup**. By the time developers query Claude Code, it's already too late - Claude has already formed incorrect assumptions about the project. Our **Proactive Context Engine** solves this by **automatically maintaining CLAUDE.md** with current project information and **implementation patterns**, ensuring Claude Code has accurate project awareness from the moment it starts.

**Core Innovation**: Proactive CLAUDE.md maintenance with implementation pattern detection prevents context issues before they happen, rather than reacting to them after the fact.

## The Real Context Problem: Information Deficit at Startup

### Current Failure Pattern
```
1. Developer starts Claude Code on project
2. Claude Code reads CLAUDE.md (missing/outdated project context)
3. Developer asks: "Add JWT validation to user endpoint"
4. Claude Code suggests generic patterns (wrong framework, wrong implementation)
5. Developer corrects Claude Code manually
6. Process repeats for every session

Problem: Claude Code lacks project awareness from the beginning
```

### Proactive Solution Pattern
```
1. Cortex detects project structure changes (package.json, code patterns, etc.)
2. Cortex automatically updates CLAUDE.md with current project context + implementation patterns
3. Developer starts Claude Code on project
4. Claude Code reads CLAUDE.md (includes current project context AND how things are implemented)
5. Developer asks: "Add JWT validation to user endpoint"
6. Claude Code suggests project-specific patterns (correct framework, correct implementation style)

Solution: Claude Code has accurate project awareness + implementation knowledge before any queries
```

## The Solution: Intelligent CLAUDE.md Maintenance with Implementation Detection

### Core Principle
> **Prevention over reaction**: Automatically maintain CLAUDE.md with current project context AND implementation patterns so Claude Code never lacks essential project awareness or implementation details.

### What We Auto-Maintain in CLAUDE.md

#### **Enhanced Project Context Section** (Auto-Generated)
```markdown
<!-- cortex:auto:begin:project-context v1 -->
## Project Context (Auto-Maintained by Cortex)
**Project Type**: Express TypeScript API
**Language**: TypeScript 5.4
**Framework**: Express.js 4.18
**Package Manager**: pnpm

### Implementation Patterns
**Authentication**: 
- Location: `src/middleware/auth.ts`
- Pattern: JWT middleware validates tokens, stores user in `req.user`
- Error Handling: Throws `AuthError` with 401 status
- Usage: Applied via `@authenticated` decorator on controllers

**Database**: 
- Location: `src/models/` (Prisma schemas), `src/repositories/` (query logic)
- Pattern: Repository pattern with `BaseRepository` class
- Connection: `src/lib/db.ts` handles Prisma client initialization
- Usage: Import repositories, call async methods with try/catch

**API Structure**:
- Location: `src/controllers/` (business logic), `src/routes/` (route definitions)
- Pattern: Controllers extend `BaseController`, routes follow `/api/v1/[resource]`
- Response Format: `{success: boolean, data?: any, error?: string}`
- Error Handling: Global error middleware catches and formats errors

**Code Conventions**:
- Naming: camelCase variables, PascalCase classes, kebab-case files
- Async: async/await throughout, no callbacks or .then()
- Imports: Relative paths for local files, absolute for node_modules
- Testing: `*.test.ts` files, Jest framework, supertest for API tests

**Key Directories**: 
  - src/services/ (business logic)
  - src/middleware/ (authentication & validation)
  - src/types/ (TypeScript interfaces)
  - src/utils/ (helper functions)

**Core Dependencies**: express, typescript, prisma, jsonwebtoken, zod, jest
**Last Updated**: 2024-01-15T10:30:00Z

*This section is automatically maintained by Cortex.*
<!-- cortex:auto:end:project-context -->
```

## Technical Architecture: Enhanced Detection + CLAUDE.md Maintenance

### **Enhanced Project Context Detector**

```typescript
interface ProjectContextInfo {
  projectType: string;
  language: string;
  framework: string;
  packageManager: string;
  directories: Array<{path: string, purpose: string}>;
  dependencies: string[];
  scripts: {dev?: string, build?: string, test?: string};
  implementationPatterns: ImplementationPatterns;
  lastUpdated: string;
}

interface ImplementationPatterns {
  authentication: AuthImplementation;
  database: DatabaseImplementation;
  api: APIImplementation;
  conventions: CodeConventions;
}

class EnhancedProjectDetector {
  detectCurrentContext(): ProjectContextInfo {
    // Basic project detection
    const pkg = this.readPackageJson();
    const basicContext = this.detectBasicProjectInfo(pkg);
    
    // Enhanced: Implementation pattern detection
    const implementationPatterns = this.detectImplementationPatterns();
    
    return {
      ...basicContext,
      implementationPatterns,
      lastUpdated: new Date().toISOString()
    };
  }
  
  private detectImplementationPatterns(): ImplementationPatterns {
    return {
      authentication: this.analyzeAuthImplementation(),
      database: this.analyzeDatabaseImplementation(),
      api: this.analyzeAPIImplementation(),
      conventions: this.analyzeCodeConventions()
    };
  }
  
  private analyzeAuthImplementation(): AuthImplementation {
    const authFiles = this.findFiles(['**/auth/**', '**/middleware/auth*', '**/guards/**']);
    const authPatterns = this.analyzeCodePatterns(authFiles);
    
    return {
      location: authFiles[0] || 'Not detected',
      pattern: this.detectAuthPattern(authPatterns), // middleware, decorator, guard
      errorHandling: this.detectErrorPattern(authPatterns),
      userStorage: this.detectUserStoragePattern(authPatterns), // req.user, context.user
      usage: this.detectAuthUsagePattern(authPatterns)
    };
  }
  
  private analyzeDatabaseImplementation(): DatabaseImplementation {
    const dbFiles = this.findFiles(['**/models/**', '**/entities/**', '**/repositories/**', '**/db/**']);
    const dbPatterns = this.analyzeCodePatterns(dbFiles);
    
    return {
      location: this.detectDatabaseLocation(dbFiles),
      pattern: this.detectDatabasePattern(dbPatterns), // ORM, query builder, raw
      connection: this.detectConnectionPattern(dbPatterns),
      usage: this.detectDatabaseUsagePattern(dbPatterns)
    };
  }
  
  private analyzeAPIImplementation(): APIImplementation {
    const apiFiles = this.findFiles(['**/routes/**', '**/controllers/**', '**/handlers/**']);
    const apiPatterns = this.analyzeCodePatterns(apiFiles);
    
    return {
      structure: this.detectAPIStructure(apiPatterns), // controller-based, file-based
      responseFormat: this.detectResponseFormat(apiPatterns),
      errorHandling: this.detectAPIErrorHandling(apiPatterns),
      routePattern: this.detectRoutePattern(apiPatterns)
    };
  }
  
  private analyzeCodeConventions(): CodeConventions {
    const codeFiles = this.findFiles(['src/**/*.ts', 'src/**/*.js']);
    const codePatterns = this.analyzeCodePatterns(codeFiles);
    
    return {
      naming: this.detectNamingConventions(codePatterns),
      asyncPattern: this.detectAsyncPattern(codePatterns),
      importStyle: this.detectImportStyle(codePatterns),
      testPattern: this.detectTestPattern(codePatterns)
    };
  }
  
  private analyzeCodePatterns(files: string[]): CodePattern[] {
    // Light AST analysis to detect implementation patterns
    // Focus on structural patterns, not full semantic analysis
    return files.map(file => {
      const content = this.readFile(file);
      return {
        file,
        exports: this.extractExports(content),
        imports: this.extractImports(content),
        classes: this.extractClasses(content),
        functions: this.extractFunctions(content),
        patterns: this.extractPatterns(content)
      };
    });
  }
}
```

### **CLAUDE.md Maintenance Engine**

```typescript
class CLAUDEMdMaintainer {
  async updateProjectContext(newContext: ProjectContextInfo): Promise<void> {
    const claudeContent = await this.readCLAUDEMd();
    const currentSection = this.extractProjectContextSection(claudeContent);
    const newSection = this.generateProjectContextSection(newContext);
    
    // Only update if there are meaningful changes
    if (this.hasSignificantChanges(currentSection, newSection)) {
      const updatedContent = this.insertOrUpdateSection(claudeContent, newSection);
      await this.atomicWrite('CLAUDE.md', updatedContent);
      console.log('✅ CLAUDE.md updated with current project context and implementation patterns');
    }
  }
  
  private generateProjectContextSection(context: ProjectContextInfo): string {
    // Safe array handling
    const directories = Array.isArray(context.directories) ? context.directories : [];
    const dependencies = Array.isArray(context.dependencies) ? context.dependencies : [];
    const scripts = context.scripts || {};
    const impl = context.implementationPatterns;
    
    const directoriesText = directories
      .map(d => `  - ${d.path} (${d.purpose})`)
      .join('\n');
      
    return `<!-- cortex:auto:begin:project-context v1 -->
## Project Context (Auto-Maintained by Cortex)
**Project Type**: ${context.projectType}
**Language**: ${context.language}
**Framework**: ${context.framework}
**Package Manager**: ${context.packageManager}

### Implementation Patterns
**Authentication**: 
- Location: \`${impl.authentication.location}\`
- Pattern: ${impl.authentication.pattern}
- Error Handling: ${impl.authentication.errorHandling}
- Usage: ${impl.authentication.usage}

**Database**: 
- Location: ${impl.database.location}
- Pattern: ${impl.database.pattern}
- Connection: ${impl.database.connection}
- Usage: ${impl.database.usage}

**API Structure**:
- Location: ${impl.api.structure}
- Response Format: ${impl.api.responseFormat}
- Error Handling: ${impl.api.errorHandling}
- Route Pattern: ${impl.api.routePattern}

**Code Conventions**:
- Naming: ${impl.conventions.naming}
- Async: ${impl.conventions.asyncPattern}
- Imports: ${impl.conventions.importStyle}
- Testing: ${impl.conventions.testPattern}

**Scripts**: dev="${scripts.dev || 'Not detected'}", build="${scripts.build || 'Not detected'}", test="${scripts.test || 'Not detected'}"
**Key Directories**: 
${directoriesText}
**Core Dependencies**: ${dependencies.join(', ')}
**Last Updated**: ${context.lastUpdated}

*This section is automatically maintained by Cortex.*
<!-- cortex:auto:end:project-context -->

`;
  }
  
  private insertOrUpdateSection(content: string, newSection: string): string {
    // Use explicit markers instead of fragile regex
    const markerStart = '<!-- cortex:auto:begin:project-context v1 -->';
    const markerEnd = '<!-- cortex:auto:end:project-context -->';
    const markerRegex = new RegExp(`${markerStart}[\\s\\S]*?${markerEnd}`, 'i');
    
    if (markerRegex.test(content)) {
      // Update existing section between markers
      return content.replace(markerRegex, newSection.trim());
    } else {
      // Insert new section after first header
      const lines = content.split('\n');
      const idx = lines.findIndex(line => line.startsWith('#'));
      const insertIndex = idx >= 0 ? idx + 1 : 0;
      lines.splice(insertIndex, 0, '', newSection.trim());
      return lines.join('\n');
    }
  }
  
  private hasSignificantChanges(currentContext: ProjectContextInfo, newContext: ProjectContextInfo): boolean {
    // Compare normalized content fingerprints
    const normalize = (context: ProjectContextInfo) => {
      const deps = Array.isArray(context.dependencies) ? context.dependencies.slice().sort() : [];
      const dirs = Array.isArray(context.directories) ? 
        context.directories.slice().sort((a, b) => a.path.localeCompare(b.path)) : [];
      
      return JSON.stringify({
        projectType: context.projectType,
        language: context.language,
        framework: context.framework,
        packageManager: context.packageManager,
        directories: dirs,
        dependencies: deps,
        implementationPatterns: context.implementationPatterns
      });
    };
    
    return normalize(currentContext) !== normalize(newContext);
  }
  
  private async atomicWrite(filePath: string, content: string): Promise<void> {
    // Atomic write to prevent corruption
    const tempPath = `${filePath}.cortex.tmp`;
    await fs.writeFile(tempPath, content, 'utf8');
    await fs.rename(tempPath, filePath);
  }
}
```

### **ContextWatcher: Enhanced File Watching System**

```typescript
class ContextWatcher {
  private detector: EnhancedProjectDetector;
  private maintainer: CLAUDEMdMaintainer;
  private debounceTimer: NodeJS.Timeout | null = null;
  
  startWatching(): void {
    // Watch both structure files AND implementation files
    const watchFiles = [
      // Project structure files
      'package.json', 'tsconfig.json', 'go.mod', 'requirements.txt', 'pyproject.toml',
      // Implementation pattern files
      'src/**/*.ts', 'src/**/*.js', 'src/**/*.py'
    ];
    
    const watcher = chokidar.watch(watchFiles, {
      ignored: [/node_modules/, /\.git/, 'CLAUDE.md'], // Exclude our own writes
      persistent: true,
      ignoreInitial: false
    });
    
    watcher.on('change', (path) => this.handleProjectFileChange(path));
    watcher.on('add', (path) => this.handleProjectFileChange(path));
    watcher.on('unlink', (path) => this.handleProjectFileChange(path));
    
    console.log('🔍 Enhanced ContextWatcher started - monitoring project structure + implementation patterns');
    
    // Initial update on startup
    this.updateContextAfterDelay();
  }
  
  private handleProjectFileChange(path: string): void {
    console.log(`📁 Project file changed: ${path}`);
    
    // Settle-based debouncing: wait for no new events for 5 seconds
    if (this.debounceTimer) {
      clearTimeout(this.debounceTimer);
    }
    
    this.debounceTimer = setTimeout(() => {
      this.updateCLAUDEMdContext();
    }, 5000);
  }
  
  private async updateCLAUDEMdContext(): Promise<void> {
    try {
      const newContext = this.detector.detectCurrentContext();
      await this.maintainer.updateProjectContext(newContext);
      console.log('✅ CLAUDE.md updated with enhanced project context');
    } catch (error) {
      console.error('❌ Failed to update CLAUDE.md context:', error);
    }
  }
}
```

## Implementation Timeline: 4-Week Enhanced Solution

### **Week 1: Enhanced Detection Engine**
- **Enhanced Project Detector**: Basic project info + implementation pattern detection
- **Pattern Analysis**: Auth, database, API, and convention detection
- **Light AST Analysis**: Extract structural patterns from code files
- **Testing**: Validate detection accuracy on current project
- **Deliverable**: Enhanced project context detection with implementation patterns

### **Week 2: CLAUDE.md Maintenance**
- **Maintenance Engine**: Generate enhanced CLAUDE.md sections with implementation details
- **Marker-based Updates**: Safe section replacement with atomic writes
- **Content Fingerprinting**: Only update when meaningful changes occur
- **Error Handling**: Graceful failures and rollback strategies
- **Deliverable**: Robust CLAUDE.md maintenance with implementation patterns

### **Week 3: Enhanced File Watching**
- **ContextWatcher**: Monitor both structure and implementation files
- **Intelligent Debouncing**: Settle-based updates during rapid changes
- **Pattern Change Detection**: Identify when implementation patterns change
- **Performance Optimization**: Efficient file monitoring and processing
- **Deliverable**: Real-time context updates for structure and implementation changes

### **Week 4: Integration & Production**
- **Architecture Integration**: Connect with existing Cortex infrastructure
- **Performance Tuning**: Optimize detection and update processes
- **Edge Case Handling**: Monorepos, missing files, permission issues
- **Monitoring & Logging**: Structured logs and health metrics
- **Deliverable**: Production-ready enhanced proactive context system

## Success Metrics: Enhanced Context Accuracy

### **Primary Success Metrics**
- **Context Freshness**: CLAUDE.md updated within 10 seconds of relevant changes
- **Implementation Accuracy**: 90% correct detection of auth, database, API patterns
- **Claude Code Accuracy**: 30% reduction in incorrect implementation suggestions
- **Developer Satisfaction**: 60% reduction in manual context corrections per session

### **Technical Performance Metrics**
- **Detection Accuracy**: 95% correct project type + 85% correct implementation patterns
- **Response Time**: Context updates complete within 5 seconds of file changes
- **System Impact**: Zero performance degradation of existing Cortex functionality
- **Reliability**: 99% uptime for enhanced file watching and maintenance

### **Real Developer Impact**
- **Better First Suggestions**: Claude Code uses project-specific patterns from startup
- **Implementation Consistency**: Suggestions match actual project implementation style
- **Reduced Explanations**: Less time spent correcting Claude Code's assumptions
- **Pattern Awareness**: Claude Code understands HOW things are done in this project

## Why This Enhanced Approach Solves the Real Problem

### **1. Addresses Amazon Q's Critique**
- **Provides HOW, not just WHAT**: Implementation patterns, not just technology stack
- **Project-specific context**: How auth works in THIS project, not generic JWT info
- **Implementation awareness**: Actual patterns used, not assumed patterns

### **2. Proactive Prevention**
- **Startup awareness**: Claude Code has full context before any queries
- **Implementation knowledge**: Knows project conventions from the beginning
- **Pattern consistency**: Suggestions match existing implementation style

### **3. Enhanced Technical Architecture**
- **Light semantic analysis**: Pattern detection without full semantic overhead
- **Robust file watching**: Monitors both structure and implementation changes
- **Safe maintenance**: Marker-based updates protect user content

### **4. Measurable Developer Impact**
- **Immediate accuracy**: First suggestions use correct patterns
- **Consistent style**: New code matches existing implementation conventions
- **Reduced friction**: Less time correcting Claude Code's generic suggestions

## Risk Mitigation

### **Technical Risks**
- **Pattern Detection Accuracy**: Start with common patterns, expand based on usage
- **File Watching Performance**: Intelligent filtering and debouncing
- **CLAUDE.md Corruption**: Marker-based updates with atomic writes

### **Implementation Risks**
- **Complexity Management**: Incremental rollout of pattern detection
- **False Positives**: Conservative pattern detection with confidence thresholds
- **Backward Compatibility**: Preserve existing CLAUDE.md content

## Enhanced Example: Before & After

### **Before: Basic CLAUDE.md**
```markdown
# CLAUDE.md

This is an Express TypeScript API project.
```

### **After: Enhanced Auto-Maintained CLAUDE.md**
```markdown
# CLAUDE.md

<!-- cortex:auto:begin:project-context v1 -->
## Project Context (Auto-Maintained by Cortex)
**Project Type**: Express TypeScript API
**Language**: TypeScript 5.4
**Framework**: Express.js 4.18
**Package Manager**: pnpm

### Implementation Patterns
**Authentication**: 
- Location: `src/middleware/auth.ts`
- Pattern: JWT middleware validates tokens, stores user in `req.user`
- Error Handling: Throws `AuthError` with 401 status
- Usage: Applied via `@authenticated` decorator on controllers

**Database**: 
- Location: `src/models/` (Prisma schemas), `src/repositories/` (query logic)
- Pattern: Repository pattern with `BaseRepository` class
- Connection: `src/lib/db.ts` handles Prisma client initialization
- Usage: Import repositories, call async methods with try/catch

**API Structure**:
- Location: `src/controllers/` (business logic), `src/routes/` (route definitions)
- Pattern: Controllers extend `BaseController`, routes follow `/api/v1/[resource]`
- Response Format: `{success: boolean, data?: any, error?: string}`
- Error Handling: Global error middleware catches and formats errors

**Code Conventions**:
- Naming: camelCase variables, PascalCase classes, kebab-case files
- Async: async/await throughout, no callbacks or .then()
- Imports: Relative paths for local files, absolute for node_modules
- Testing: `*.test.ts` files, Jest framework, supertest for API tests

**Key Directories**: 
  - src/services/ (business logic)
  - src/middleware/ (authentication & validation)
  - src/types/ (TypeScript interfaces)
  - src/utils/ (helper functions)

**Core Dependencies**: express, typescript, prisma, jsonwebtoken, zod, jest
**Last Updated**: 2024-01-15T10:30:00Z

*This section is automatically maintained by Cortex.*
<!-- cortex:auto:end:project-context -->

## Development Guidelines
[Existing user content preserved...]
```

## Conclusion: Complete Context Solution

The **Enhanced Cortex Proactive Context Engine** solves Claude Code's context accuracy problems by providing both **project awareness** and **implementation pattern knowledge** through intelligent CLAUDE.md maintenance. This addresses Amazon Q's critique by delivering not just WHAT technologies are used, but HOW they are implemented in this specific project.

### **Key Innovation: Prevention + Implementation Awareness**

#### **1. Proactive CLAUDE.md Maintenance**
- **Project structure detection** for basic awareness
- **Implementation pattern analysis** for specific knowledge
- **Automatic updates** when patterns change

#### **2. Enhanced Context Delivery**
- **Technology stack** - What frameworks are used
- **Implementation patterns** - How they are actually used
- **Code conventions** - Project-specific styles and patterns
- **Usage examples** - How to follow existing patterns

#### **3. Complete Solution**
- **Prevents wrong assumptions** through startup awareness
- **Provides implementation guidance** through pattern detection
- **Maintains accuracy** through real-time updates
- **Preserves user content** through safe maintenance

### **Expected Impact**

**Before Enhanced Context**:
- Claude Code suggests generic JWT middleware
- Uses assumed database patterns
- Proposes standard API responses
- Follows generic conventions

**After Enhanced Context**:
- **Claude Code suggests THIS project's JWT middleware pattern**
- **Uses THIS project's repository pattern**
- **Proposes THIS project's response format**
- **Follows THIS project's naming conventions**

### **The Complete Solution**

By combining proactive CLAUDE.md maintenance with implementation pattern detection, we ensure Claude Code never lacks both **project awareness** (what kind of project) and **implementation knowledge** (how things are done in this project).

**Result**: Claude Code that **inherently understands your project's architecture AND implementation patterns** - providing suggestions that **always align with your existing codebase style and conventions**.

---

**Next Steps**: Begin Week 1 implementation of enhanced project detection to provide Claude Code with **complete project awareness and implementation pattern knowledge**.