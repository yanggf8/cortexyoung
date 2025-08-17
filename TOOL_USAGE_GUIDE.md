# 🛠️ CORTEX MCP TOOLS - USAGE GUIDE FOR CLAUDE CODE

## 🎯 **TOOL SELECTION STRATEGY**

### **Quick Reference: When to Use Which Tool**

| Task Type | Primary Tool | Secondary Tool | MMR Preset |
|-----------|-------------|----------------|------------|
| **"Find a bug"** | `semantic_search` | `trace_execution_path` | `high-relevance` |
| **"Understand this system"** | `code_intelligence` | `relationship_analysis` | `high-diversity` |
| **"Implement a feature"** | `code_intelligence` | `find_code_patterns` | `balanced` |
| **"Refactor safely"** | `relationship_analysis` | `find_code_patterns` | `high-relevance` |
| **"Learn new codebase"** | `code_intelligence` | `semantic_search` | `high-diversity` |
| **"Debug specific issue"** | `semantic_search` | `trace_execution_path` | `high-relevance` |
| **"Architecture review"** | `relationship_analysis` | `find_code_patterns` | `high-diversity` |

---

## 🔧 **TOOL CAPABILITIES & BEST PRACTICES**

### **1. `semantic_search` - Quick Code Discovery**
**🎯 BEST FOR**: Finding specific functions, debugging, exploring code patterns
**⚡ STRENGTHS**: 
- Fast semantic search with MMR optimization
- Automatic dependency inclusion
- Real-time file watching integration

**💡 USAGE TIPS**:
```typescript
// For debugging
semantic_search({
  query: "error handling for authentication failures",
  multi_hop: { enabled: true, max_hops: 2 }
})

// For exploring patterns  
semantic_search({
  query: "database connection management",
  include_related: true,
  max_chunks: 15
})
```

### **2. `code_intelligence` - Complex Analysis**
**🎯 BEST FOR**: High-level development tasks, architecture understanding
**⚡ STRENGTHS**: 
- Comprehensive semantic analysis
- Smart dependency chain traversal
- Critical set protection (95%+ essential code coverage)
- Context window optimization

**💡 USAGE TIPS**:
```typescript
// For feature implementation
code_intelligence({
  task: "implement user authentication with JWT tokens",
  max_context_tokens: 8000,
  recency_weight: 0.4  // Include recent changes
})

// For system understanding
code_intelligence({
  task: "understand the payment processing workflow",
  focus_areas: ["payment", "billing", "transactions"],
  max_context_tokens: 12000
})
```

### **3. `relationship_analysis` - Dependency Mapping**
**🎯 BEST FOR**: Impact analysis, refactoring, understanding code connections
**⚡ STRENGTHS**: 
- Advanced relationship traversal
- Strength scoring and confidence metrics
- Multiple visualization formats
- Deep dependency analysis

**💡 USAGE TIPS**:
```typescript
// For refactoring impact analysis
relationship_analysis({
  analysis_type: "impact_analysis",
  starting_symbols: ["UserService.authenticate"],
  max_depth: 4,
  include_strength_scores: true
})

// For understanding call chains
relationship_analysis({
  analysis_type: "call_graph", 
  starting_symbols: ["processPayment"],
  relationship_filters: ["calls", "data_flow"],
  visualization_format: "mermaid"
})
```

### **4. `trace_execution_path` - Flow Analysis**
**🎯 BEST FOR**: Understanding execution flow, debugging complex interactions
**⚡ STRENGTHS**: 
- Forward/backward/bidirectional tracing
- Data flow analysis
- Error path detection

**💡 USAGE TIPS**:
```typescript
// For debugging execution flow
trace_execution_path({
  entry_point: "handleUserLogin",
  trace_type: "bidirectional",
  include_error_paths: true,
  max_execution_depth: 6
})
```

### **5. `find_code_patterns` - Pattern Recognition**
**🎯 BEST FOR**: Code quality analysis, architectural patterns, anti-patterns
**⚡ STRENGTHS**: 
- Design pattern detection
- Anti-pattern identification
- Architectural pattern analysis

**💡 USAGE TIPS**:
```typescript
// For finding design patterns
find_code_patterns({
  pattern_type: "design_pattern",
  pattern_description: "observer pattern for event handling",
  confidence_threshold: 0.8
})

// For code smell detection
find_code_patterns({
  pattern_type: "code_smell",
  pattern_description: "large classes with too many responsibilities",
  scope: "entire_codebase"
})
```

---

## 🎛️ **MMR OPTIMIZATION SETTINGS**

### **Preset Selection Guide**:

**`balanced` (λ=0.7)** - DEFAULT
- **Use for**: General queries, feature development, mixed tasks
- **Balance**: 70% relevance, 30% diversity
- **Best when**: You need focused but comprehensive context

**`high-relevance` (λ=0.9)** - FOCUSED  
- **Use for**: Debugging, specific function analysis, targeted searches
- **Balance**: 90% relevance, 10% diversity
- **Best when**: You know exactly what you're looking for

**`high-diversity` (λ=0.4)** - EXPLORATORY
- **Use for**: Learning new codebases, architecture review, comprehensive analysis
- **Balance**: 40% relevance, 60% diversity  
- **Best when**: You need broad understanding of the system

### **Setting MMR via Environment Variables**:
```bash
# For focused debugging sessions
export CORTEX_MMR_LAMBDA=0.9

# For exploration and learning
export CORTEX_MMR_LAMBDA=0.4

# Increase token budget for complex analysis
export CORTEX_MMR_TOKEN_BUDGET=120000
```

---

## 🚀 **TOOL COMBINATION STRATEGIES**

### **🔄 Progressive Analysis Workflow**:

1. **Start Broad** → `code_intelligence` (high-diversity)
2. **Focus Down** → `semantic_search` (balanced) 
3. **Deep Dive** → `relationship_analysis` (high-relevance)
4. **Trace Flow** → `trace_execution_path` (balanced)

### **🎯 Task-Specific Combinations**:

**For Bug Fixing**:
```
semantic_search("error symptoms") → 
trace_execution_path(suspected_function) → 
relationship_analysis(impact_analysis)
```

**For Feature Implementation**:
```
code_intelligence("feature requirements") → 
find_code_patterns("similar implementations") → 
relationship_analysis("integration points")
```

**For Code Review**:
```
find_code_patterns("code_smell") → 
relationship_analysis("dependency_cycle") → 
trace_execution_path("critical_paths")
```

---

## 📊 **CONTEXT WINDOW OPTIMIZATION TIPS**

### **🎯 Maximize Information per Token**:

1. **Use specific queries**: "JWT token validation logic" vs "authentication"
2. **Leverage multi-hop**: Enable relationship traversal for complete context
3. **Set appropriate token budgets**: 4K for focused, 12K for comprehensive
4. **Use file filters**: Narrow scope when you know the area
5. **Check real-time status**: Ensure you're working with fresh data

### **🔍 Query Quality Guidelines**:

**✅ GOOD QUERIES**:
- "error handling in payment processing"
- "database connection pool management" 
- "user session validation middleware"

**❌ AVOID**:
- "code" (too broad)
- "function" (too generic)
- "bug" (not specific enough)

### **📈 Token Budget Recommendations**:

| Task Complexity | Token Budget | Expected Context |
|-----------------|-------------|------------------|
| **Quick lookup** | 2,000-4,000 | 5-10 relevant functions |
| **Feature analysis** | 6,000-10,000 | Complete feature context |
| **System understanding** | 10,000-16,000 | Architectural overview |
| **Complex debugging** | 8,000-12,000 | Full error propagation chain |

---

## ⚡ **REAL-TIME FEATURES**

### **Always Check Freshness**:
```typescript
// Before starting complex analysis
real_time_status()  // Verify index is up-to-date
```

**Real-time benefits**:
- ✅ Context includes latest code changes
- ✅ Dependency analysis reflects current state  
- ✅ No stale information affecting decisions

---

## 🎯 **SUCCESS METRICS TO WATCH**

The system tracks context effectiveness automatically:
- **Follow-up rate**: Lower = better initial context
- **Critical set coverage**: Higher = more complete context
- **Token efficiency**: Higher = better information density
- **Reference match rate**: Higher = more useful context

**Use `npm run telemetry:dashboard` to see your optimization progress!**