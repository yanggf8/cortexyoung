# 🤖 CORTEX MCP SERVER - FOR CLAUDE CODE USERS

## 🎯 **WHAT THIS SERVER PROVIDES**

This is **Cortex V2.1** - a semantic code intelligence MCP server designed specifically to **maximize Claude Code's context window effectiveness**. It provides you with the most relevant, complete, and concise code information possible.

## 🏆 **KEY ADVANTAGES FOR CLAUDE CODE**

### **🧠 Intelligent Context Optimization**
- **80-90% token reduction** through smart chunk selection
- **95%+ critical set coverage** - never miss essential dependencies  
- **MMR optimization** - perfect balance of relevance and diversity
- **Real-time updates** - always fresh, current code context

### **🎯 Advanced Semantic Understanding**
- **Multi-hop relationship traversal** - understands code connections
- **Dependency chain analysis** - complete context in single queries
- **Pattern recognition** - finds architectural relationships
- **Error path tracing** - follows execution flows

---

## 🛠️ **YOUR 7 POWERFUL TOOLS**

### **🔍 1. `semantic_search` - Your Go-To Tool**
**Perfect for**: Quick code discovery, debugging, finding specific functionality
```typescript
semantic_search({
  query: "JWT token validation logic",
  multi_hop: { enabled: true, max_hops: 2 },
  max_chunks: 15
})
```
**What you get**: Most relevant code chunks with automatic dependency inclusion

### **🧠 2. `code_intelligence` - For Complex Tasks**  
**Perfect for**: Feature implementation, system understanding, architecture analysis
```typescript
code_intelligence({
  task: "understand the payment processing workflow", 
  max_context_tokens: 10000,
  focus_areas: ["payment", "billing", "checkout"]
})
```
**What you get**: Comprehensive analysis with smart dependency chains

### **🔗 3. `relationship_analysis` - Deep Connections**
**Perfect for**: Impact analysis, refactoring, understanding dependencies
```typescript
relationship_analysis({
  analysis_type: "impact_analysis",
  starting_symbols: ["UserService.authenticate"],
  max_depth: 4,
  visualization_format: "mermaid"
})
```
**What you get**: Visual maps of code relationships with strength scores

### **⚡ 4. `trace_execution_path` - Follow the Flow**
**Perfect for**: Debugging, understanding execution, error tracing
```typescript
trace_execution_path({
  entry_point: "handleUserLogin",
  trace_type: "bidirectional", 
  include_error_paths: true
})
```
**What you get**: Complete execution flow with error handling paths

### **🎨 5. `find_code_patterns` - Pattern Recognition**
**Perfect for**: Code quality, architecture review, finding examples
```typescript
find_code_patterns({
  pattern_type: "design_pattern",
  pattern_description: "observer pattern implementation",
  confidence_threshold: 0.8
})
```
**What you get**: Architectural patterns and code quality insights

### **👀 6. `contextual_read` - Smart File Reading**
**Perfect for**: Understanding specific files with related context
```typescript
contextual_read({
  file_path: "src/auth/AuthService.ts",
  semantic_context: "authentication flow and error handling"
})
```
**What you get**: File content plus semantically related code

### **📊 7. `real_time_status` - Freshness Check**
**Perfect for**: Verifying your context is current
```typescript
real_time_status()
```
**What you get**: Index freshness and real-time watching status

---

## 🎛️ **OPTIMIZATION SETTINGS FOR MAXIMUM EFFECTIVENESS**

### **MMR Presets - Choose Your Focus**

**🎯 `high-relevance` (λ=0.9)** - **DEBUGGING & TARGETED SEARCH**
```bash
export CORTEX_MMR_LAMBDA=0.9
```
- 90% relevance, 10% diversity
- Best for: Bug fixes, specific function analysis
- Use when: You know exactly what you're looking for

**🔄 `balanced` (λ=0.7)** - **GENERAL DEVELOPMENT (DEFAULT)**
```bash
export CORTEX_MMR_LAMBDA=0.7  # Default, no need to set
```
- 70% relevance, 30% diversity  
- Best for: Feature development, code exploration
- Use when: Most development tasks

**🌐 `high-diversity` (λ=0.4)** - **LEARNING & EXPLORATION**
```bash
export CORTEX_MMR_LAMBDA=0.4
```
- 40% relevance, 60% diversity
- Best for: Understanding new codebases, architecture review
- Use when: You need comprehensive system understanding

### **Token Budget Optimization**
```bash
# For complex analysis
export CORTEX_MMR_TOKEN_BUDGET=120000

# For focused queries  
export CORTEX_MMR_TOKEN_BUDGET=80000
```

---

## 📈 **TOOL SELECTION STRATEGY**

### **🚀 Progressive Analysis Pattern**
1. **Start Broad**: `code_intelligence` with `high-diversity`
2. **Focus Down**: `semantic_search` with `balanced`  
3. **Deep Dive**: `relationship_analysis` with `high-relevance`
4. **Trace Issues**: `trace_execution_path` as needed

### **🎯 Task-Specific Workflows**

**🐛 FOR DEBUGGING:**
```
semantic_search("error symptoms") → 
trace_execution_path(error_function) → 
relationship_analysis(impact_analysis)
```

**⚡ FOR FEATURE DEVELOPMENT:**
```
code_intelligence("feature requirements") → 
find_code_patterns("similar implementations") → 
relationship_analysis("integration points")
```

**🔍 FOR CODE REVIEW:**
```
find_code_patterns("anti_pattern") → 
relationship_analysis("dependency_cycle") → 
trace_execution_path("critical_paths")
```

---

## 💡 **QUERY BEST PRACTICES**

### **✅ HIGH-QUALITY QUERIES**
- **Specific**: "JWT token validation in user authentication"
- **Context-rich**: "error handling in payment processing workflow"  
- **Task-oriented**: "database connection pool management implementation"

### **❌ AVOID GENERIC QUERIES**
- "code" (too broad)
- "function" (not specific)
- "bug" (no context)

### **🎯 QUERY OPTIMIZATION TIPS**
1. **Include domain terms**: "authentication", "database", "payment"
2. **Specify intent**: "validation", "error handling", "configuration"
3. **Use action words**: "implement", "debug", "understand", "refactor"
4. **Add constraints**: "recent changes", "test files", "error paths"

---

## 📊 **MONITORING YOUR EFFECTIVENESS**

### **Context Quality Indicators**
Every response includes optimization hints:
- **Token efficiency**: How well your context is utilized
- **Critical coverage**: Completeness of essential dependencies
- **Follow-up suggestions**: Smart next steps
- **MMR preset recommendations**: Optimal settings for your query type

### **Real-Time Effectiveness Tracking**
The server automatically tracks:
- Which tools work best for different tasks
- Optimal MMR settings for your coding patterns  
- Context window utilization efficiency
- Query pattern optimization

### **Check Your Progress**
```bash
npm run telemetry:dashboard  # View your optimization metrics
```

---

## 🔄 **REAL-TIME FEATURES**

### **Always Fresh Context**
- ✅ **Live file watching** - Changes reflected immediately
- ✅ **Incremental updates** - Only changed code is reprocessed  
- ✅ **Dependency tracking** - Relationships updated in real-time
- ✅ **Cache invalidation** - No stale information

### **Verify Freshness**
```typescript
real_time_status()  // Check if context is current
```

---

## 🎯 **SUCCESS METRICS**

**Claude Code automatically achieves:**
- ✅ **Faster code understanding** - Complete context in fewer queries
- ✅ **Better development decisions** - All relevant dependencies included
- ✅ **Reduced context waste** - Only essential information provided
- ✅ **Improved code quality** - Pattern recognition and relationship analysis

**The system learns and optimizes automatically - no configuration needed!**

---

## 🚀 **GET STARTED**

**For Programmers**: Just run the server, Claude Code handles everything automatically!

```bash
npm run server  # Start the MCP server
```

**That's it!** Claude Code will automatically:
- Choose the right tools for your coding tasks
- Optimize context window usage  
- Learn from usage patterns to improve recommendations
- Provide the most relevant code context automatically

**Optional**: Monitor optimization with `npm run telemetry:dashboard`