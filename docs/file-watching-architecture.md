# File Watching Architecture Design
## Cortex V2.1 Real-Time Code Intelligence System

### 🎯 **Objective**
Transform Cortex into a **real-time code intelligence platform** with instant index updates, sub-second search responses, and adaptive file monitoring.

---

## 🏗️ **System Architecture Overview**

### **Core Components**

```
┌─────────────────────────────────────────────────────────────┐
│                    File Watching System                     │
├─────────────────────────────────────────────────────────────┤
│  ┌─────────────────┐  ┌─────────────────┐  ┌─────────────────┐ │
│  │   File Watcher  │  │ Activity Detector│  │ Change Processor│ │
│  │   (chokidar)    │  │  (adaptive)     │  │   (debounced)   │ │
│  └─────────────────┘  └─────────────────┘  └─────────────────┘ │
└─────────────────────────────────────────────────────────────┘
                                │
                                ▼
┌─────────────────────────────────────────────────────────────┐
│                 Incremental Index Engine                    │
├─────────────────────────────────────────────────────────────┤
│  ┌─────────────────┐  ┌─────────────────┐  ┌─────────────────┐ │
│  │  Delta Calculator│  │ Embedding Queue │  │ Relationship    │ │
│  │   (enhanced)    │  │  (prioritized)  │  │   Updater       │ │
│  └─────────────────┘  └─────────────────┘  └─────────────────┘ │
└─────────────────────────────────────────────────────────────┘
                                │
                                ▼
┌─────────────────────────────────────────────────────────────┐
│                   Live Search Engine                        │
├─────────────────────────────────────────────────────────────┤
│  ┌─────────────────┐  ┌─────────────────┐  ┌─────────────────┐ │
│  │   Hot Index     │  │  Search Cache   │  │   MCP Bridge    │ │
│  │  (in-memory)    │  │  (invalidated)  │  │  (real-time)    │ │
│  └─────────────────┘  └─────────────────┘  └─────────────────┘ │
└─────────────────────────────────────────────────────────────┘
```

---

## 📁 **File Watching Layer**

### **1. FileWatcher Class**
```typescript
interface FileWatcherConfig {
  repositoryPath: string;
  ignorePatterns: string[];
  debounceMs: number;
  batchSize: number;
  adaptiveThreshold: number;
}

interface FileChangeEvent {
  type: 'add' | 'change' | 'unlink' | 'addDir' | 'unlinkDir';
  path: string;
  timestamp: number;
  size?: number;
  stats?: fs.Stats;
}

class FileWatcher extends EventEmitter {
  private watcher: chokidar.FSWatcher;
  private activityDetector: ActivityDetector;
  private changeProcessor: ChangeProcessor;
  private config: FileWatcherConfig;
  
  async initialize(): Promise<void>;
  async start(): Promise<void>;
  async stop(): Promise<void>;
  
  // Event emissions
  emit(event: 'fileChange', changes: FileChangeEvent[]): boolean;
  emit(event: 'activityStateChange', state: ActivityState): boolean;
}
```

### **2. Activity Detection System**
```typescript
interface ActivityState {
  isActive: boolean;
  intensity: 'low' | 'medium' | 'high';
  changeRate: number; // changes per second
  lastActivity: Date;
  suspendProcessing: boolean;
}

class ActivityDetector {
  private changeHistory: Map<string, number[]> = new Map();
  private activityWindow: number = 30000; // 30 seconds
  private thresholds = {
    low: 2,    // < 2 changes/30s
    medium: 10, // 2-10 changes/30s  
    high: 20   // > 10 changes/30s
  };
  
  recordChange(filePath: string): void;
  getActivityState(): ActivityState;
  shouldSuspendProcessing(): boolean;
}
```

### **3. Change Processing Pipeline**
```typescript
class ChangeProcessor {
  private changeQueue: PriorityQueue<FileChangeEvent>;
  private debounceTimers: Map<string, NodeJS.Timeout>;
  private batchProcessor: BatchProcessor;
  
  async processChange(event: FileChangeEvent): Promise<void>;
  private async debouncedProcess(filePath: string): Promise<void>;
  private async batchProcess(changes: FileChangeEvent[]): Promise<void>;
}
```

---

## ⚡ **Incremental Index Engine**

### **1. Enhanced Delta Calculator**
```typescript
interface LiveIndexDelta extends IndexDelta {
  priority: 'high' | 'medium' | 'low';
  estimatedProcessingTime: number;
  dependencies: string[]; // related files that might need updates
  changeType: 'content' | 'structure' | 'metadata';
}

class LiveDeltaCalculator extends PersistentVectorStore {
  async calculateLiveDelta(
    changes: FileChangeEvent[]
  ): Promise<LiveIndexDelta>;
  
  private async analyzeChangeImpact(
    filePath: string, 
    changeType: string
  ): Promise<{
    affectedChunks: string[];
    relatedFiles: string[];
    priority: 'high' | 'medium' | 'low';
  }>;
  
  private async prioritizeChanges(
    deltas: LiveIndexDelta[]
  ): Promise<LiveIndexDelta[]>;
}
```

### **2. Prioritized Embedding Queue**
```typescript
interface EmbeddingTask {
  id: string;
  chunks: CodeChunk[];
  priority: number;
  filePath: string;
  estimatedTime: number;
  dependencies: string[];
  retryCount: number;
}

class PrioritizedEmbeddingQueue {
  private queue: PriorityQueue<EmbeddingTask>;
  private processing: Map<string, EmbeddingTask>;
  private embedder: IEmbedder;
  private maxConcurrent: number = 3;
  
  async enqueue(task: EmbeddingTask): Promise<void>;
  async processNext(): Promise<void>;
  async processAll(): Promise<void>;
  
  // Priority calculation
  private calculatePriority(task: EmbeddingTask): number;
  private async estimateProcessingTime(chunks: CodeChunk[]): Promise<number>;
}
```

### **3. Live Relationship Updates**
```typescript
class LiveRelationshipUpdater {
  private relationshipStore: PersistentRelationshipStore;
  private dependencyMapper: DependencyMapper;
  
  async updateRelationships(
    changedFiles: string[],
    delta: LiveIndexDelta
  ): Promise<void>;
  
  private async identifyAffectedRelationships(
    filePath: string
  ): Promise<string[]>;
  
  private async incrementalRelationshipUpdate(
    filePath: string,
    content: string
  ): Promise<void>;
}
```

---

## 🔍 **Live Search Engine**

### **1. Hot Index Management**
```typescript
interface HotIndexEntry {
  chunkId: string;
  embedding: number[];
  metadata: any;
  lastUpdated: Date;
  version: number;
}

class HotIndex {
  private memoryIndex: Map<string, HotIndexEntry>;
  private persistentStore: PersistentVectorStore;
  private maxMemorySize: number = 1000; // chunks
  
  async updateChunk(chunk: CodeChunk): Promise<void>;
  async removeChunk(chunkId: string): Promise<void>;
  async search(query: number[], limit: number): Promise<CodeChunk[]>;
  
  // Memory management
  private async evictOldEntries(): Promise<void>;
  private async syncToPersistent(): Promise<void>;
}
```

### **2. Invalidation-Based Search Cache**
```typescript
interface SearchCacheEntry {
  query: string;
  results: QueryResponse;
  timestamp: Date;
  affectedFiles: Set<string>;
  version: number;
}

class InvalidationSearchCache {
  private cache: Map<string, SearchCacheEntry>;
  private fileToQueries: Map<string, Set<string>>;
  
  async get(query: string): Promise<QueryResponse | null>;
  async set(query: string, results: QueryResponse, affectedFiles: string[]): Promise<void>;
  async invalidateByFiles(changedFiles: string[]): Promise<void>;
  async invalidateAll(): Promise<void>;
}
```

---

## 🔄 **Integration Points**

### **1. Enhanced CodebaseIndexer**
```typescript
class LiveCodebaseIndexer extends CodebaseIndexer {
  private fileWatcher: FileWatcher;
  private hotIndex: HotIndex;
  private embeddingQueue: PrioritizedEmbeddingQueue;
  private relationshipUpdater: LiveRelationshipUpdater;
  private searchCache: InvalidationSearchCache;
  
  async enableLiveMode(): Promise<void>;
  async disableLiveMode(): Promise<void>;
  
  // Event handlers
  private async onFileChange(changes: FileChangeEvent[]): Promise<void>;
  private async onActivityStateChange(state: ActivityState): Promise<void>;
  
  // Live indexing pipeline
  private async processLiveChanges(changes: FileChangeEvent[]): Promise<void>;
  private async updateHotIndex(delta: LiveIndexDelta): Promise<void>;
}
```

### **2. Enhanced Server Integration**
```typescript
// In server.ts
class CortexMCPServer {
  private liveIndexer: LiveCodebaseIndexer;
  private isLiveModeEnabled: boolean = false;
  
  async enableLiveMode(): Promise<void> {
    if (!this.isLiveModeEnabled) {
      await this.liveIndexer.enableLiveMode();
      this.isLiveModeEnabled = true;
      log('🔴 Live mode enabled - real-time code intelligence active');
    }
  }
  
  async disableLiveMode(): Promise<void> {
    if (this.isLiveModeEnabled) {
      await this.liveIndexer.disableLiveMode();
      this.isLiveModeEnabled = false;
      log('⚪ Live mode disabled - static indexing mode');
    }
  }
}
```

---

## 📊 **Performance Considerations**

### **1. Memory Management**
- **Hot Index Size Limit**: 1000 most recent/frequently accessed chunks in memory
- **LRU Eviction**: Least recently used chunks moved to persistent storage
- **Memory Monitoring**: Automatic scaling based on available system memory

### **2. Processing Efficiency**
- **Debouncing**: 500ms default debounce for rapid file changes
- **Batching**: Process up to 10 files per batch to optimize embedding generation
- **Priority Queue**: High-priority files (currently open, recently searched) processed first

### **3. Activity Adaptation**
```typescript
interface AdaptiveConfig {
  lowActivity: {
    debounceMs: 200;
    batchSize: 5;
    processingDelay: 0;
  };
  mediumActivity: {
    debounceMs: 1000;
    batchSize: 10;
    processingDelay: 2000;
  };
  highActivity: {
    debounceMs: 5000;
    batchSize: 20;
    processingDelay: 10000; // Wait for activity to calm down
  };
}
```

---

## 🛡️ **Error Handling & Resilience**

### **1. Graceful Degradation**
- **Watcher Failure**: Fall back to periodic scanning
- **Embedding Errors**: Retry with exponential backoff, skip problematic files
- **Memory Pressure**: Reduce hot index size, increase persistence frequency

### **2. Consistency Guarantees**
- **Atomic Updates**: All index changes are atomic (success or rollback)
- **Version Tracking**: Each chunk has a version number for conflict resolution
- **Conflict Resolution**: Last-write-wins with timestamp comparison

### **3. Recovery Mechanisms**
```typescript
class LiveIndexRecovery {
  async detectInconsistencies(): Promise<string[]>;
  async repairIndex(inconsistentFiles: string[]): Promise<void>;
  async fullResync(): Promise<void>;
  
  // Health monitoring
  async validateIndexHealth(): Promise<{
    healthy: boolean;
    issues: string[];
    recommendedAction: 'continue' | 'repair' | 'rebuild';
  }>;
}
```

---

## 🚀 **Implementation Phases**

### **Phase 1: Core File Watching** (Week 1)
- [ ] Implement `FileWatcher` class with chokidar
- [ ] Create `ActivityDetector` for adaptive behavior
- [ ] Build `ChangeProcessor` with debouncing and batching
- [ ] Add basic file change event handling

### **Phase 2: Live Delta Processing** (Week 2)
- [ ] Enhance `PersistentVectorStore` with live delta calculation
- [ ] Implement `PrioritizedEmbeddingQueue`
- [ ] Create `LiveRelationshipUpdater`
- [ ] Add priority-based change processing

### **Phase 3: Hot Index & Search** (Week 3)
- [ ] Implement `HotIndex` for in-memory chunk storage
- [ ] Create `InvalidationSearchCache` with file-based invalidation
- [ ] Integrate with existing search pipeline
- [ ] Add real-time search capabilities

### **Phase 4: Integration & Polish** (Week 4)
- [ ] Integrate with `CodebaseIndexer` and `CortexMCPServer`
- [ ] Add comprehensive error handling and recovery
- [ ] Implement performance monitoring and adaptive scaling
- [ ] Create configuration management and user controls

---

## 🎛️ **Configuration Options**

### **Environment Variables**
```bash
# Live mode settings
CORTEX_LIVE_MODE=true
CORTEX_WATCH_DEBOUNCE_MS=500
CORTEX_WATCH_BATCH_SIZE=10
CORTEX_HOT_INDEX_SIZE=1000

# Activity detection
CORTEX_ACTIVITY_WINDOW_MS=30000
CORTEX_HIGH_ACTIVITY_THRESHOLD=20
CORTEX_SUSPEND_ON_HIGH_ACTIVITY=true

# Performance tuning
CORTEX_MAX_CONCURRENT_EMBEDDINGS=3
CORTEX_MEMORY_LIMIT_MB=2048
CORTEX_CACHE_TTL_MS=300000
```

### **Runtime Controls**
```typescript
// MCP tool for live mode control
{
  name: "toggle_live_mode",
  description: "Enable or disable real-time file watching",
  inputSchema: {
    type: "object",
    properties: {
      enabled: { type: "boolean" },
      config: { 
        type: "object",
        properties: {
          debounceMs: { type: "number" },
          batchSize: { type: "number" },
          hotIndexSize: { type: "number" }
        }
      }
    }
  }
}
```

---

## 📈 **Success Metrics**

### **Performance Targets**
- **Index Update Latency**: < 2 seconds for single file changes
- **Search Response Time**: < 500ms for cached queries, < 2s for new queries
- **Memory Usage**: < 500MB additional overhead for live mode
- **CPU Impact**: < 10% additional CPU usage during normal activity

### **Reliability Targets**
- **Uptime**: 99.9% availability during live mode
- **Consistency**: 100% eventual consistency within 30 seconds
- **Error Recovery**: Automatic recovery from 95% of transient failures

---

## 🔮 **Future Enhancements**

### **Advanced Features**
- **Semantic Change Detection**: Detect when changes affect code semantics vs. formatting
- **Predictive Indexing**: Pre-index files likely to be modified based on patterns
- **Collaborative Intelligence**: Share index updates across team members
- **IDE Integration**: Direct integration with VS Code, IntelliJ, etc.

### **Performance Optimizations**
- **Incremental Embeddings**: Update embeddings for changed chunks only
- **Distributed Processing**: Scale embedding generation across multiple machines
- **GPU Acceleration**: Use GPU for faster embedding computation
- **Smart Caching**: ML-based cache prediction and preloading

---

This architecture provides a solid foundation for transforming Cortex into a real-time code intelligence platform while maintaining the existing performance optimizations and reliability features.