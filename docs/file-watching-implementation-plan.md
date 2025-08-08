# Smart File Watching Implementation Plan - APPROVED

**Status**: Expert-reviewed and approved for implementation  
**Timeline**: 12 weeks (MVP-first approach)  
**Expert Review**: Gemini validated architecture and timeline  

## 🎯 **Project Overview**

Transform Cortex V2.1 into a real-time code intelligence system with smart file watching that adapts to development activity patterns while maintaining excellent performance characteristics.

### **Two-Phase Strategy**
- **Phase A (Weeks 1-6)**: Smart MVP with fixed debouncing and proven patterns
- **Phase B (Weeks 7-12)**: Advanced adaptive features built on stable foundation

**Benefits**: Delivers 80% value with 30% complexity, reducing implementation risk while providing immediate user value.

---

## 📋 **PHASE A: Smart MVP (Weeks 1-6)**

### **Week 1-2: Core Infrastructure**
**Goal**: Basic file watching with proven patterns

#### Core Implementation Tasks:
1. **SmartFileWatcher MVP** - chokidar integration with git-tracked filtering
2. **Fixed Debouncing** - 3-5 second configurable timeout (not hardcoded)
3. **Change Queue** - File-level coalescing (multiple changes = single task)
4. **Configuration System** - Runtime configurable thresholds via config file
5. **Error Handling** - Graceful degradation and comprehensive logging

#### Technical Specifications:
```typescript
interface WatcherConfig {
  enabled: boolean;
  debounceMs: number;        // Fixed 3000ms (3s) for MVP
  batchSize: number;         // Default: 50
  maxQueueSize: number;      // Default: 200
  overflowStrategy: 'coalesce' | 'drop-oldest' | 'pause';
  watchPaths: string[];      // ['src/', 'lib/']
  ignorePatterns: string[];  // ['node_modules/**', 'dist/**']
}

class SmartFileWatcher {
  // Fixed debouncing for MVP reliability
  private debounceMs = 3000; // Configurable, not adaptive yet
  private changeQueue = new Map<string, FileChange>(); // File-level coalescing
}
```

### **Week 3-4: Integration (CRITICAL PHASE)**
**Goal**: Integrate with existing Cortex architecture  
**Risk Level**: HIGH - Allocated 2 weeks instead of 1

#### **CRITICAL: Delta Analysis Strategy**
**Decision Point**: Prototype both approaches, choose based on performance data

```typescript
// Option 1: File-Level Processing (MVP Default)
interface FileChangeStrategy {
  processFileChange(filePath: string): Promise<void>;
  // Re-chunk and re-embed entire modified file
}

// Option 2: AST-Level Diffing (Prototype Only)
interface ASTDiffStrategy {
  processASTDiff(filePath: string, changes: ASTDiff[]): Promise<void>;
  // Only process changed functions/classes
}
```

**Timeboxing Strategy**: 
- **Days 1-4**: AST approach prototype (MAXIMUM 4 days)
- **Day 5+**: Fallback to file-level if AST not viable

#### Integration Tasks:
1. **UnifiedStorageCoordinator Integration** - Incremental updates with existing system
2. **ProcessPoolEmbedder Integration** - Backpressure handling for queue saturation
3. **Idempotent Storage** - Retry logic for failed storage operations
4. **Git Event Detection** - Detect branch switching (>500 files in <10s)
5. **Delta Analysis Enhancement** - Work with real-time file changes

#### Technical Details:
```typescript
class GitEventDetector {
  detectMassiveChange(changes: FileChange[]): 'branch-switch' | 'normal' {
    if (changes.length > 500 && this.timeSpan(changes) < 10000) {
      return 'branch-switch'; // Pause watcher, wait for settling
    }
    return 'normal';
  }
}

class BackpressureHandler {
  async handleEmbedderSaturation(): Promise<void> {
    // Queue changes when ProcessPoolEmbedder at capacity
    // Resume processing when resources available
  }
}
```

### **Week 5: Testing & Edge Cases**
**Goal**: Comprehensive validation and cross-platform compatibility

#### Testing Tasks:
1. **Unit Tests** - Change coalescing, queue management, config validation
2. **Integration Tests** - Real file changes, git operations, storage updates
3. **Cross-Platform Testing** - Windows/macOS/Linux file system differences
4. **File Rename Detection** - Avoid delete+add cycles for renames
5. **Queue Overflow Handling** - Coalesce vs drop strategies
6. **Structured Logging** - JSON format with rich context for debugging

#### Edge Case Handling:
```typescript
class EdgeCaseHandler {
  handleFileRename(oldPath: string, newPath: string): void {
    // Detect rename pattern: delete + add within time window
    // Treat as rename, preserve embeddings and relationships
  }
  
  handleQueueOverflow(): void {
    // Coalesce changes by file path
    // Drop oldest changes if still over limit
    // Log overflow events for monitoring
  }
}
```

### **Week 6: User Interface & Documentation**
**Goal**: Production-ready system with user controls

#### User Interface Tasks:
1. **CLI Commands** - start/stop/pause/status functionality
2. **NPM Scripts** - Integration with existing build system
3. **Status Reporting** - Queue metrics, activity levels, pause reasons
4. **Manual Triggers** - Force processing of queued changes
5. **Server Integration** - Add as Stage 11 in startup process

#### NPM Scripts Integration:
```json
{
  "scripts": {
    "watch:start": "node -e \"require('./dist/src/watcher-cli').startWatcher()\"",
    "watch:stop": "node -e \"require('./dist/src/watcher-cli').stopWatcher()\"",
    "watch:pause": "node -e \"require('./dist/src/watcher-cli').pauseWatcher(300)\"",
    "watch:status": "node -e \"require('./dist/src/watcher-cli').showStatus()\"",
    "watch:process": "node -e \"require('./dist/src/watcher-cli').triggerProcessing()\"",
    "dev:watch": "CORTEX_WATCH_ENABLED=true npm run server"
  }
}
```

---

## 📊 **PHASE B: Advanced Intelligence (Weeks 7-12)**

### **Week 7-8: Activity Monitoring**
**Goal**: Adaptive behavior based on real MVP usage data

#### Advanced Features:
1. **ActivityMonitor Class** - Pattern analysis from real-world usage
2. **Adaptive Debouncing** - idle: 2s, normal: 10s, heavy: 30s
3. **Intelligent Batching** - Priority levels based on activity
4. **Usage Analytics** - Inform adaptive algorithm design

### **Week 9-10: Advanced Processing**
**Goal**: Sophisticated resource management and strategy selection

#### Advanced Features:
1. **AdaptiveProcessor** - Strategy selection based on system state
2. **Cooperative Yielding** - During heavy coding sessions
3. **Predictive Scaling** - Resource usage prediction
4. **Circuit Breaker** - Advanced error resilience patterns

### **Week 11: Production Features**
**Goal**: Enterprise-grade monitoring and security

#### Production Features:
1. **Comprehensive Metrics** - Prometheus/DataDog integration
2. **Security Controls** - File access validation and resource limits
3. **Health Monitoring** - Circuit breaker and auto-recovery
4. **Advanced MCP Tools** - Enhanced Claude Code integration

### **Week 12: Final Integration**
**Goal**: Production deployment and validation

#### Final Tasks:
1. **Performance Testing** - Load testing and benchmarking
2. **Documentation** - User guides and troubleshooting
3. **Migration Tools** - Smooth transition from manual indexing
4. **Production Deployment** - Docker configs and monitoring setup

---

## 🛡️ **Risk Mitigation Strategies**

### **Technical Risks**
| Risk | Phase | Mitigation |
|------|-------|------------|
| Delta Analysis Complexity | A-3 | Prototype both approaches, timebox AST to 4 days |
| ProcessPoolEmbedder Integration | A-3 | Explicit backpressure handling and queuing |
| Cross-Platform Issues | A-5 | Dedicated testing on all platforms |
| Storage Race Conditions | A-3 | Idempotent operations with retry logic |
| Performance Impact | A-6 | MVP validation with real usage data |

### **Timeline Risks**
| Phase | Risk Level | Buffer Strategy |
|-------|------------|----------------|
| A-3 (Integration) | HIGH | 2 weeks allocated (doubled from original) |
| A-5 (Testing) | MEDIUM | Comprehensive testing week |
| B-1-B-2 (Intelligence) | LOW | Built on proven MVP foundation |

---

## ⚙️ **Configuration Architecture**

### **Environment Variables**
```bash
# Basic file watching
CORTEX_WATCH_ENABLED=true
CORTEX_WATCH_DEBOUNCE_MS=3000      # Fixed for MVP (Phase A)
CORTEX_WATCH_BATCH_SIZE=50
CORTEX_WATCH_MAX_QUEUE_SIZE=200

# Git event detection
CORTEX_WATCH_GIT_THRESHOLD=500     # Files to trigger branch switch detection
CORTEX_WATCH_GIT_TIME_WINDOW=10000 # Time window in ms

# Paths and patterns
CORTEX_WATCH_PATHS=src/,lib/,components/
CORTEX_WATCH_IGNORE=node_modules/**,dist/**,.git/**

# Advanced features (Phase B)
CORTEX_WATCH_ADAPTIVE_ENABLED=false  # Enable adaptive debouncing
CORTEX_WATCH_ACTIVITY_IDLE=5         # Changes per minute
CORTEX_WATCH_ACTIVITY_HEAVY=20       # Changes per minute
```

### **Runtime Configuration File**
```json
{
  "fileWatcher": {
    "enabled": true,
    "debouncing": {
      "fixed": 3000,
      "adaptive": {
        "enabled": false,
        "idle": 2000,
        "normal": 10000,  
        "heavy": 30000
      }
    },
    "queue": {
      "maxSize": 200,
      "overflowStrategy": "coalesce"
    },
    "gitEvents": {
      "massChangeThreshold": 500,
      "timeWindow": 10000
    }
  }
}
```

---

## 🎯 **Success Metrics**

### **Phase A MVP Success Criteria**
- ✅ **File change detection**: < 100ms response for status queries
- ✅ **Debouncing reliability**: Consistent 3-5s fixed debounce
- ✅ **Queue management**: Handle 200+ changes with coalescing
- ✅ **Git awareness**: Detect and handle branch switching
- ✅ **Cross-platform**: Work on Windows/macOS/Linux
- ✅ **Memory overhead**: < 50MB additional memory usage
- ✅ **Zero process leaks**: No orphaned watcher processes
- ✅ **Production stability**: 24/7 operation without crashes

### **Phase B Advanced Success Criteria**
- ✅ **Activity detection**: Accurately classify idle/normal/heavy
- ✅ **Adaptive performance**: Responsive during idle, stable during heavy
- ✅ **Resource prediction**: Prevent system overload
- ✅ **Advanced monitoring**: Rich metrics and alerting
- ✅ **User experience**: Invisible intelligence during development

---

## 📚 **Implementation Notes**

### **Key Dependencies**
```json
{
  "dependencies": {
    "chokidar": "^3.5.3",      // File watching
    "fastq": "^1.15.0"         // Queue management (existing)
  },
  "devDependencies": {
    "@types/chokidar": "^2.1.3"
  }
}
```

### **File Structure**
```
src/
├── file-watching/
│   ├── smart-file-watcher.ts      // Main watcher orchestrator
│   ├── change-coalescer.ts        // File-level change management
│   ├── git-event-detector.ts      // Branch switch detection
│   ├── backpressure-handler.ts    // ProcessPool integration
│   ├── watcher-cli.ts             // Command-line interface
│   └── activity-monitor.ts        // Phase B: Activity analysis
├── server.ts                      // Enhanced with Stage 11
└── indexer.ts                     // Enhanced with real-time updates
```

### **Expert Validation Summary**

**Gemini's Assessment**: 
- ✅ **"Excellent revision - MVP-first approach dramatically de-risks the project"**
- ✅ **"Timeline is realistic and well-structured"**
- ✅ **"Technical choices are sound, pragmatic, and wise"**
- ✅ **"Architecture provides scalable foundation"**
- ✅ **"Phase A delivers production-ready system"**

**Recommendation**: **"Proceed with this plan"**

---

## 🚀 **Ready for Implementation**

This plan successfully addresses all critical concerns identified in the expert review:
- ✅ Extended timeline to realistic 12 weeks
- ✅ MVP-first approach reduces complexity and risk  
- ✅ All technical risks have explicit mitigation strategies
- ✅ Configuration system prevents hardcoded values
- ✅ Cross-platform compatibility explicitly addressed
- ✅ Integration challenges allocated appropriate time

**The plan is approved and ready for Phase A implementation!** 🎯