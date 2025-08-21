# Cortex Goals: Proactive Project Awareness for Claude Code

## Primary Mission

Transform Claude Code from a context-lacking assistant to a project-aware development partner through **proactive project awareness** - ensuring Claude Code understands the project's implementation patterns from startup, before any queries are asked.

## The 80/20 Problem We're Solving

### 🎯 **80% Impact (What We're Fixing)**
- Claude Code suggestions that break existing architecture
- Missing critical dependencies causing runtime errors  
- Code inconsistent with project patterns and conventions
- Development friction and eroded trust in AI assistance
- Constant need to correct Claude Code's architectural misconceptions

### 🔍 **20% Root Cause (What We're Targeting)**
**Lack of project awareness at startup** - Claude Code doesn't know how the project implements authentication, API responses, and error handling before developers ask questions.

## Core Goals

### Goal 1: **Proactive Implementation Pattern Detection**
**Automatically detect HOW the project implements critical patterns before Claude Code needs them**

**What This Means:**
- Detect how authentication actually works (`req.user` vs `req.context.user`, cookie vs header tokens)
- Discover actual API response format (`{data: any}` vs `{result: any}` vs bare objects)
- Identify real error handling patterns (`{error: string}` vs `{message: string}`, global middleware vs try/catch)
- Focus on implementation details that prevent wrong suggestions

**Success Criteria:**
- 85%+ confidence in detecting authentication user property patterns
- 90%+ accuracy in identifying API response wrapper formats
- 80%+ confidence in error handling structure detection
- Evidence-based detection with file path citations

### Goal 2: **Startup Project Awareness**
**Ensure Claude Code knows project implementation patterns from the moment it starts**

**What This Means:**
- Proactive CLAUDE.md maintenance with implementation patterns before first query
- Boot-time project awareness that prevents wrong assumptions
- Critical guardrails clearly documented (e.g., "NEVER use localStorage for tokens")
- Prevention approach: context ready before problems happen

**Success Criteria:**
- CLAUDE.md updated within 10 seconds of project changes
- Implementation patterns documented before first developer interaction
- Critical guardrails prevent common architectural mistakes
- Zero manual context preparation required

### Goal 3: **Implementation Detail Accuracy**
**Focus on the specific implementation details that cause wrong suggestions**

**What This Means:**
- Not just "uses JWT" but "JWT in httpOnly cookies, user in req.user, 401 errors"
- Not just "REST API" but "{data: any} wrapper, explicit status codes, always wrapped responses"
- Not just "error handling" but "global middleware, {error: string} format, throw exceptions"
- Deep implementation knowledge over broad technology detection

**Success Criteria:**
- Prevent localStorage suggestions in cookie-based auth projects
- Prevent bare object responses in wrapper-based API projects  
- Prevent inconsistent error formats across the codebase
- 30% reduction in implementation-specific wrong suggestions

### Goal 4: **Confidence-Based Reliability**
**Only assert implementation patterns when confident, mark uncertainty clearly**

**What This Means:**
- 60%+ confidence threshold for asserting implementation patterns
- "Unknown" states for uncertain patterns to prevent wrong guidance
- Evidence citations for all implementation pattern claims
- Low-confidence warnings: "Ask before making assumptions"

**Success Criteria:**
- No false positive implementation guidance that breaks existing patterns
- Clear uncertainty indicators when pattern detection is ambiguous
- Evidence-backed claims with file paths supporting each assertion
- Developer trust in pattern detection accuracy

## Implementation Strategy

### Phase 1: Implementation Pattern Detection Engine
**Build the core implementation pattern detection and analysis system**

**Components:**
- **Authentication Pattern Detection**: Extract user property patterns, token storage, error formats
- **API Response Pattern Detection**: Identify success/error formats, wrapper patterns, status code usage
- **Error Handling Pattern Detection**: Discover catch patterns, error structures, propagation styles
- **Evidence Collection**: File content analysis with citation tracking

### Phase 2: Proactive CLAUDE.md Maintenance
**Develop automatic project awareness system that prevents context issues**

**Components:**
- **Boot-time Context Generation**: Populate CLAUDE.md before first developer interaction
- **Pattern Documentation**: Generate implementation-specific guidance with confidence scores
- **Critical Guardrails**: Document project-specific constraints and patterns
- **Atomic Updates**: Safe marker-based content updates with error recovery

### Phase 3: Real-time Project Awareness Maintenance
**Create self-updating project awareness that evolves with codebase changes**

**Components:**
- **Pattern Change Detection**: Monitor implementation file changes for pattern updates
- **Context Freshness**: Keep CLAUDE.md current with codebase evolution
- **Confidence Tracking**: Maintain accuracy of implementation pattern detection
- **Reliability Monitoring**: Ensure consistent project awareness quality

## Success Metrics

### Implementation Pattern Accuracy Metrics
- **Authentication Detection**: 85%+ confidence in user property patterns (`req.user` vs `req.context.user`)
- **API Response Detection**: 90%+ accuracy in response format detection (`{data: any}` vs bare objects)
- **Error Handling Detection**: 80%+ confidence in error structure patterns (`{error: string}` vs `{message: string}`)
- **Evidence Quality**: All pattern assertions backed by file citations and confidence scores

### Startup Awareness Metrics  
- **Boot-time Readiness**: CLAUDE.md populated with implementation patterns before first interaction
- **Context Freshness**: Implementation patterns updated within 10 seconds of relevant file changes
- **Guardrail Effectiveness**: Critical warnings prevent common implementation mistakes
- **Zero Manual Preparation**: No developer action required for project awareness

### Prevention Effectiveness Metrics
- **Wrong Suggestion Reduction**: 30% fewer implementation-specific incorrect suggestions from Claude Code
- **Pattern Consistency**: 95%+ adherence to detected implementation patterns in suggestions
- **First Interaction Accuracy**: Correct implementation guidance from the very first Claude Code query
- **Architecture Alignment**: Zero localStorage suggestions in cookie-based auth projects

## Vision: Implementation-Aware Claude Code

**The Future State**: Claude Code that knows exactly HOW your project implements authentication, API responses, and error handling from the moment it starts - preventing wrong implementation suggestions before they happen.

**The Developer Experience**: Work with Claude Code that immediately understands your project's specific patterns - suggesting `req.user` instead of `req.context.user`, `{data: any}` instead of bare objects, and never recommending localStorage in cookie-based auth projects.

**The Technical Reality**: A proactive implementation pattern detection engine that automatically maintains CLAUDE.md with deep project awareness, ensuring Claude Code has accurate implementation knowledge before any developer interaction.

---

**These goals transform Cortex from a reactive search tool into a proactive project awareness engine that solves the real problem: giving Claude Code startup implementation knowledge to prevent wrong suggestions from the first interaction.**