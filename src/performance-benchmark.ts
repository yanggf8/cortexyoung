import { CodebaseIndexer } from './indexer';
import { SemanticSearcher } from './searcher';
import { HierarchicalStageTracker } from './hierarchical-stages';
import { UnifiedStorageCoordinator } from './unified-storage-coordinator';
import * as fs from 'fs/promises';
import * as path from 'path';
import * as os from 'os';
import { performance } from 'perf_hooks';

interface BenchmarkResult {
  name: string;
  startTime: number;
  endTime: number;
  duration: number;
  success: boolean;
  details?: any;
  error?: string;
  memoryUsage?: NodeJS.MemoryUsage;
  chunksProcessed?: number;
  throughputChunksPerSecond?: number;
}

interface PerformanceReport {
  timestamp: number;
  systemInfo: {
    platform: string;
    arch: string;
    nodeVersion: string;
    totalMemory: number;
    cpuCount: number;
  };
  repositoryInfo: {
    path: string;
    fileCount: number;
    totalSizeBytes: number;
    gitCommitCount: number;
  };
  benchmarks: {
    startup: BenchmarkResult[];
    indexing: BenchmarkResult[];
    search: BenchmarkResult[];
    storage: BenchmarkResult[];
  };
  summary: {
    totalDuration: number;
    successRate: number;
    memoryPeakMB: number;
    recommendedActions: string[];
  };
}

export class PerformanceBenchmark {
  private repositoryPath: string;
  private results: BenchmarkResult[] = [];
  private startupTracker: HierarchicalStageTracker;
  private peakMemoryUsage = 0;

  constructor(repositoryPath: string) {
    this.repositoryPath = repositoryPath;
    this.startupTracker = new HierarchicalStageTracker();
  }

  private recordMemoryUsage(): NodeJS.MemoryUsage {
    const usage = process.memoryUsage();
    const totalMB = (usage.heapUsed + usage.external) / 1024 / 1024;
    if (totalMB > this.peakMemoryUsage) {
      this.peakMemoryUsage = totalMB;
    }
    return usage;
  }

  private async runBenchmark<T>(
    name: string,
    operation: () => Promise<T>
  ): Promise<{ result: T; benchmark: BenchmarkResult }> {
    console.log(`🏃 Running benchmark: ${name}`);
    
    const startTime = performance.now();
    const startMemory = this.recordMemoryUsage();
    
    try {
      const result = await operation();
      const endTime = performance.now();
      const endMemory = this.recordMemoryUsage();
      
      const benchmark: BenchmarkResult = {
        name,
        startTime,
        endTime,
        duration: endTime - startTime,
        success: true,
        memoryUsage: endMemory
      };
      
      this.results.push(benchmark);
      const durationText = `${benchmark.duration.toFixed(2)}ms`;
      const throughputText = benchmark.throughputChunksPerSecond 
        ? ` (${benchmark.chunksProcessed} chunks, ${benchmark.throughputChunksPerSecond.toFixed(2)} chunks/sec)`
        : '';
      console.log(`✅ ${name}: ${durationText}${throughputText}`);
      
      return { result, benchmark };
    } catch (error) {
      const endTime = performance.now();
      const benchmark: BenchmarkResult = {
        name,
        startTime,
        endTime,
        duration: endTime - startTime,
        success: false,
        error: error instanceof Error ? error.message : 'Unknown error'
      };
      
      this.results.push(benchmark);
      console.error(`❌ ${name}: Failed after ${benchmark.duration.toFixed(2)}ms - ${benchmark.error}`);
      
      throw error;
    }
  }

  async runStartupBenchmarks(): Promise<BenchmarkResult[]> {
    console.log('🚀 Running Startup Performance Benchmarks');
    console.log('========================================');
    
    const startupBenchmarks: BenchmarkResult[] = [];
    
    // 1. Cold start (no cache)
    await this.runBenchmark('cold-start-cache-clear', async () => {
      const coordinator = new UnifiedStorageCoordinator(this.repositoryPath);
      await coordinator.initialize();
      await coordinator.clearAll();
    });
    
    const { result: coldStartResult, benchmark: coldStart } = await this.runBenchmark('cold-start-full-index', async () => {
      const indexer = new CodebaseIndexer(this.repositoryPath);
      return await indexer.indexRepository({
        repository_path: this.repositoryPath,
        mode: 'full'
      });
    });
    
    // Calculate real throughput: chunks processed / wall clock seconds
    coldStart.chunksProcessed = coldStartResult.chunks_processed;
    coldStart.throughputChunksPerSecond = (coldStart.chunksProcessed || 0) / (coldStart.duration / 1000);
    startupBenchmarks.push(coldStart);
    
    // 2. Warm start (with cache)
    const { result: warmStartResult, benchmark: warmStart } = await this.runBenchmark('warm-start-incremental', async () => {
      const indexer = new CodebaseIndexer(this.repositoryPath);
      return await indexer.indexRepository({
        repository_path: this.repositoryPath,
        mode: 'incremental'
      });
    });
    
    // Calculate real throughput for warm start
    warmStart.chunksProcessed = warmStartResult.chunks_processed;
    warmStart.throughputChunksPerSecond = (warmStart.chunksProcessed || 0) / (warmStart.duration / 1000);
    startupBenchmarks.push(warmStart);
    
    // 3. Cache-only start (no changes)
    const { result: cacheStartResult, benchmark: cacheStart } = await this.runBenchmark('cache-only-start', async () => {
      const indexer = new CodebaseIndexer(this.repositoryPath);
      return await indexer.indexRepository({
        repository_path: this.repositoryPath,
        mode: 'incremental'
      });
    });
    
    // Calculate real throughput for cache-only start
    cacheStart.chunksProcessed = cacheStartResult.chunks_processed;
    cacheStart.throughputChunksPerSecond = (cacheStart.chunksProcessed || 0) / (cacheStart.duration / 1000);
    startupBenchmarks.push(cacheStart);
    
    return startupBenchmarks;
  }

  async runSearchBenchmarks(): Promise<BenchmarkResult[]> {
    console.log('🔍 Running Search Performance Benchmarks');
    console.log('======================================');
    
    const searchBenchmarks: BenchmarkResult[] = [];
    
    // Initialize indexer and searcher
    const indexer = new CodebaseIndexer(this.repositoryPath);
    await indexer.indexRepository({
      repository_path: this.repositoryPath,
      mode: 'incremental'
    });
    
    const searcher = (indexer as any).searcher;
    
    // Test queries of varying complexity
    const testQueries = [
      {
        name: 'simple-search',
        query: 'function definition',
        maxChunks: 10
      },
      {
        name: 'complex-search-with-relationships',
        query: 'error handling patterns with logging',
        maxChunks: 20,
        multiHop: {
          enabled: true,
          max_hops: 2,
          relationship_types: ['calls', 'imports', 'data_flow']
        }
      },
      {
        name: 'large-result-search',
        query: 'typescript interface class',
        maxChunks: 50
      },
      {
        name: 'relationship-heavy-search',
        query: 'MCP server implementation',
        maxChunks: 30,
        multiHop: {
          enabled: true,
          max_hops: 3,
          relationship_types: ['calls', 'imports', 'data_flow', 'co_change']
        }
      }
    ];
    
    for (const testQuery of testQueries) {
      const { benchmark } = await this.runBenchmark(`search-${testQuery.name}`, async () => {
        return await searcher.search({
          task: testQuery.query,
          max_chunks: testQuery.maxChunks,
          context_mode: 'structured',
          multi_hop: testQuery.multiHop
        });
      });
      
      searchBenchmarks.push(benchmark);
    }
    
    return searchBenchmarks;
  }

  async runStorageBenchmarks(): Promise<BenchmarkResult[]> {
    console.log('💾 Running Storage Performance Benchmarks');
    console.log('========================================');
    
    const storageBenchmarks: BenchmarkResult[] = [];
    const coordinator = new UnifiedStorageCoordinator(this.repositoryPath);
    await coordinator.initialize();
    
    // Storage operations
    const { benchmark: statusCheck } = await this.runBenchmark('storage-status-check', async () => {
      return await coordinator.getStorageStatus();
    });
    storageBenchmarks.push(statusCheck);
    
    const { benchmark: statsGeneration } = await this.runBenchmark('storage-stats-generation', async () => {
      return await coordinator.getStorageStats();
    });
    storageBenchmarks.push(statsGeneration);
    
    const { benchmark: consistencyValidation } = await this.runBenchmark('storage-consistency-validation', async () => {
      return await coordinator.validateConsistency();
    });
    storageBenchmarks.push(consistencyValidation);
    
    const { benchmark: syncOperation } = await this.runBenchmark('storage-sync-operation', async () => {
      return await coordinator.syncAll();
    });
    storageBenchmarks.push(syncOperation);
    
    return storageBenchmarks;
  }

  private async getRepositoryInfo() {
    const files = await this.getAllFiles(this.repositoryPath);
    const stats = await Promise.all(
      files.map(f => fs.stat(f).catch(() => ({ size: 0 })))
    );
    const totalSize = stats.reduce((sum, stat) => sum + stat.size, 0);
    
    return {
      path: this.repositoryPath,
      fileCount: files.length,
      totalSizeBytes: totalSize,
      gitCommitCount: 0 // Could implement git log counting
    };
  }

  private async getAllFiles(dir: string): Promise<string[]> {
    const files: string[] = [];
    const entries = await fs.readdir(dir, { withFileTypes: true });
    
    for (const entry of entries) {
      const fullPath = path.join(dir, entry.name);
      if (entry.name.startsWith('.')) continue;
      
      if (entry.isDirectory()) {
        files.push(...await this.getAllFiles(fullPath));
      } else if (entry.isFile() && /\.(ts|js|tsx|jsx)$/.test(entry.name)) {
        files.push(fullPath);
      }
    }
    
    return files;
  }

  async generatePerformanceReport(): Promise<PerformanceReport> {
    console.log('📊 Generating Performance Report');
    console.log('===============================');
    
    const repositoryInfo = await this.getRepositoryInfo();
    
    // Categorize benchmarks
    const startupBenchmarks = this.results.filter(r => 
      r.name.includes('start') || r.name.includes('index')
    );
    const searchBenchmarks = this.results.filter(r => r.name.includes('search'));
    const storageBenchmarks = this.results.filter(r => r.name.includes('storage'));
    const indexingBenchmarks = this.results.filter(r => 
      r.name.includes('index') && !r.name.includes('start')
    );
    
    const totalDuration = this.results.reduce((sum, r) => sum + r.duration, 0);
    const successCount = this.results.filter(r => r.success).length;
    const successRate = (successCount / this.results.length) * 100;
    
    // Generate recommendations
    const recommendations: string[] = [];
    const coldStartTime = startupBenchmarks.find(b => b.name.includes('cold-start-full'))?.duration || 0;
    const warmStartTime = startupBenchmarks.find(b => b.name.includes('warm-start'))?.duration || 0;
    
    if (coldStartTime > 120000) { // > 2 minutes
      recommendations.push('Consider optimizing embedding generation batch size');
    }
    if (warmStartTime > 5000) { // > 5 seconds
      recommendations.push('Cache loading might need optimization');
    }
    if (this.peakMemoryUsage > 1000) { // > 1GB
      recommendations.push('Memory usage is high, consider chunk size optimization');
    }
    if (successRate < 100) {
      recommendations.push('Some benchmarks failed, check error logs');
    }
    
    // Check for timeout-related issues
    const hasTimeoutIssues = this.results.some(r => r.error && r.error.includes('timeout'));
    if (hasTimeoutIssues) {
      recommendations.push('Timeout issues detected - progressive timeout system should prevent SIGKILL errors');
    }
    
    // Check embedding throughput
    const embeddingBenchmarks = this.results.filter(r => r.throughputChunksPerSecond && r.throughputChunksPerSecond > 0);
    const avgThroughput = embeddingBenchmarks.length > 0 
      ? embeddingBenchmarks.reduce((sum, r) => sum + (r.throughputChunksPerSecond || 0), 0) / embeddingBenchmarks.length
      : 0;
    
    if (avgThroughput > 0 && avgThroughput < 10) {
      recommendations.push('Embedding throughput below target - consider batch size optimization');
    } else if (avgThroughput >= 12) {
      recommendations.push('Excellent embedding throughput achieved');
    }
    
    return {
      timestamp: Date.now(),
      systemInfo: {
        platform: os.platform(),
        arch: os.arch(),
        nodeVersion: process.version,
        totalMemory: os.totalmem(),
        cpuCount: os.cpus().length
      },
      repositoryInfo,
      benchmarks: {
        startup: startupBenchmarks,
        indexing: indexingBenchmarks,
        search: searchBenchmarks,
        storage: storageBenchmarks
      },
      summary: {
        totalDuration,
        successRate,
        memoryPeakMB: this.peakMemoryUsage,
        recommendedActions: recommendations
      }
    };
  }

  async runFullBenchmarkSuite(): Promise<PerformanceReport> {
    console.log('🏁 Starting Full Performance Benchmark Suite');
    console.log('============================================');
    
    const overallStart = performance.now();
    
    try {
      // Run all benchmark categories
      await this.runStartupBenchmarks();
      await this.runSearchBenchmarks();
      await this.runStorageBenchmarks();
      
      const report = await this.generatePerformanceReport();
      const overallDuration = performance.now() - overallStart;
      
      console.log('\n📊 Benchmark Suite Complete');
      console.log(`⏱️  Total Time: ${(overallDuration / 1000).toFixed(2)}s`);
      console.log(`✅ Success Rate: ${report.summary.successRate.toFixed(1)}%`);
      console.log(`🧠 Peak Memory: ${report.summary.memoryPeakMB.toFixed(1)}MB`);
      
      // Show throughput for startup benchmarks
      const startupWithThroughput = report.benchmarks.startup.filter(b => b.throughputChunksPerSecond && b.throughputChunksPerSecond > 0);
      if (startupWithThroughput.length > 0) {
        console.log('\n🚀 Startup Throughput Summary:');
        startupWithThroughput.forEach(b => {
          console.log(`   ${b.name}: ${b.chunksProcessed} chunks in ${(b.duration/1000).toFixed(1)}s = ${b.throughputChunksPerSecond?.toFixed(2)} chunks/sec`);
        });
      }
      
      return report;
    } catch (error) {
      console.error('❌ Benchmark suite failed:', error);
      throw error;
    }
  }

  async saveReport(report: PerformanceReport, filename?: string): Promise<string> {
    const reportsDir = path.join(this.repositoryPath, 'performance-reports');
    await fs.mkdir(reportsDir, { recursive: true });
    
    const timestamp = new Date().toISOString().replace(/[:.]/g, '-');
    const reportFile = filename || `performance-report-${timestamp}.json`;
    const reportPath = path.join(reportsDir, reportFile);
    
    await fs.writeFile(reportPath, JSON.stringify(report, null, 2));
    console.log(`📄 Performance report saved: ${reportPath}`);
    
    return reportPath;
  }
}