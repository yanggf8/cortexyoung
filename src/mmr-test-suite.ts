import { GuardedMMRSelector, MMRConfig } from './guarded-mmr-selector';
import { MMRConfigManager } from './mmr-config-manager';
import { CodeChunk, QueryRequest } from './types';
import { log, warn, error } from './logging-utils';
import { performance } from 'perf_hooks';

export interface MMRTestCase {
  name: string;
  description: string;
  query: QueryRequest;
  mockChunks: CodeChunk[];
  expectedMetrics: {
    minCriticalSetCoverage: number;
    maxSelectionTime: number;
    maxTokens: number;
    minDiversityScore: number;
  };
}

export interface MMRTestResult {
  testCase: string;
  passed: boolean;
  actualMetrics: {
    criticalSetCoverage: number;
    selectionTime: number;
    totalTokens: number;
    diversityScore: number;
    chunksSelected: number;
  };
  expectedMetrics: {
    minCriticalSetCoverage: number;
    maxSelectionTime: number;
    maxTokens: number;
    minDiversityScore: number;
  };
  issues: string[];
  performance: {
    avgSelectionTime: number;
    p95SelectionTime: number;
    throughputChunksPerSecond: number;
  };
}

export class MMRTestSuite {
  private selector: GuardedMMRSelector;
  private configManager: MMRConfigManager;

  constructor(config?: Partial<MMRConfig>) {
    this.selector = new GuardedMMRSelector(config);
    this.configManager = new MMRConfigManager();
  }

  private createTestChunks(count: number, filePrefix: string = 'test'): CodeChunk[] {
    const chunks: CodeChunk[] = [];
    
    for (let i = 0; i < count; i++) {
      const embedding = new Array(384).fill(0).map(() => Math.random() * 2 - 1);
      
      chunks.push({
        chunk_id: `chunk_${i}`,
        file_path: `${filePrefix}_${Math.floor(i / 5)}.ts`,
        symbol_name: `symbol_${i}`,
        function_name: `function_${i}`,
        chunk_type: i % 4 === 0 ? 'function' : i % 4 === 1 ? 'class' : i % 4 === 2 ? 'method' : 'documentation',
        start_line: i * 10,
        end_line: i * 10 + 10,
        content: `// Test content for chunk ${i}\nfunction test_${i}() {\n  return "test_${i}";\n}`.repeat(10 + i % 5),
        content_hash: `hash_${i}`,
        embedding,
        relationships: {
          calls: i > 0 ? [`symbol_${i-1}`] : [],
          called_by: i < count - 1 ? [`symbol_${i+1}`] : [],
          imports: [`import_${i}`],
          exports: [`export_${i}`],
          data_flow: []
        },
        git_metadata: {
          last_modified_commit: `commit_${i}`,
          commit_author: 'test_author',
          commit_message: `Test commit ${i}`,
          commit_date: new Date(Date.now() - i * 24 * 60 * 60 * 1000).toISOString(),
          file_history_length: 10,
          co_change_files: []
        },
        language_metadata: {
          language: 'typescript',
          complexity_score: 1 + (i % 5),
          dependencies: [`dep_${i}`],
          exports: [`export_${i}`]
        },
        usage_patterns: {
          access_frequency: Math.random(),
          task_contexts: [`context_${i}`]
        },
        last_modified: new Date().toISOString(),
        similarity_score: Math.random(),
        relevance_score: Math.random()
      });
    }
    
    return chunks;
  }

  private getTestCases(): MMRTestCase[] {
    return [
      {
        name: 'small_focused_query',
        description: 'Small query with specific file and function mentions',
        query: {
          task: 'Find the getUserData function in user.ts file',
          max_chunks: 10
        },
        mockChunks: this.createTestChunks(25, 'user'),
        expectedMetrics: {
          minCriticalSetCoverage: 0.8,
          maxSelectionTime: 50,
          maxTokens: 15000,
          minDiversityScore: 0.3
        }
      },
      {
        name: 'large_diverse_query',
        description: 'Large query requiring diverse chunks from multiple files',
        query: {
          task: 'Analyze the authentication system including database operations, middleware, and error handling',
          max_chunks: 50
        },
        mockChunks: this.createTestChunks(200, 'auth'),
        expectedMetrics: {
          minCriticalSetCoverage: 0.7,
          maxSelectionTime: 200,
          maxTokens: 80000,
          minDiversityScore: 0.5
        }
      },
      {
        name: 'critical_set_heavy',
        description: 'Query with many explicit file and function mentions',
        query: {
          task: 'Review handleLogin, validateUser, and checkPermissions functions in auth.ts, user.ts, and permissions.ts files',
          max_chunks: 30
        },
        mockChunks: this.createTestChunks(100, 'system'),
        expectedMetrics: {
          minCriticalSetCoverage: 0.9,
          maxSelectionTime: 100,
          maxTokens: 50000,
          minDiversityScore: 0.2
        }
      },
      {
        name: 'memory_constrained',
        description: 'Test with very small token budget',
        query: {
          task: 'Quick review of error handling patterns',
          max_chunks: 20
        },
        mockChunks: this.createTestChunks(150, 'error'),
        expectedMetrics: {
          minCriticalSetCoverage: 0.6,
          maxSelectionTime: 80,
          maxTokens: 25000,
          minDiversityScore: 0.4
        }
      },
      {
        name: 'performance_stress',
        description: 'Large candidate set stress test',
        query: {
          task: 'Comprehensive codebase analysis for refactoring opportunities',
          max_chunks: 100
        },
        mockChunks: this.createTestChunks(1000, 'large'),
        expectedMetrics: {
          minCriticalSetCoverage: 0.5,
          maxSelectionTime: 500, // Allow more time for large dataset
          maxTokens: 150000,
          minDiversityScore: 0.6
        }
      }
    ];
  }

  async runSingleTest(testCase: MMRTestCase): Promise<MMRTestResult> {
    const issues: string[] = [];
    const iterations = testCase.mockChunks.length > 500 ? 3 : 10; // Fewer iterations for large datasets
    const selectionTimes: number[] = [];

    let lastResult;
    
    try {
      // Run multiple iterations for performance measurement
      for (let i = 0; i < iterations; i++) {
        const startTime = performance.now();
        lastResult = await this.selector.selectOptimalChunks(
          testCase.mockChunks,
          testCase.query
        );
        const endTime = performance.now();
        selectionTimes.push(endTime - startTime);
      }

      if (!lastResult) {
        throw new Error('No result returned from MMR selection');
      }

      // Validate metrics
      if (lastResult.criticalSetCoverage < testCase.expectedMetrics.minCriticalSetCoverage) {
        issues.push(`Critical set coverage too low: ${(lastResult.criticalSetCoverage * 100).toFixed(1)}% < ${(testCase.expectedMetrics.minCriticalSetCoverage * 100).toFixed(1)}%`);
      }

      if (lastResult.selectionTime > testCase.expectedMetrics.maxSelectionTime) {
        issues.push(`Selection time too high: ${lastResult.selectionTime.toFixed(2)}ms > ${testCase.expectedMetrics.maxSelectionTime}ms`);
      }

      if (lastResult.totalTokens > testCase.expectedMetrics.maxTokens) {
        issues.push(`Token count too high: ${lastResult.totalTokens} > ${testCase.expectedMetrics.maxTokens}`);
      }

      if (lastResult.diversityScore < testCase.expectedMetrics.minDiversityScore) {
        issues.push(`Diversity score too low: ${lastResult.diversityScore.toFixed(3)} < ${testCase.expectedMetrics.minDiversityScore}`);
      }

      // Performance metrics
      const avgSelectionTime = selectionTimes.reduce((sum, time) => sum + time, 0) / selectionTimes.length;
      const sortedTimes = selectionTimes.sort((a, b) => a - b);
      const p95SelectionTime = sortedTimes[Math.floor(sortedTimes.length * 0.95)];
      const throughputChunksPerSecond = testCase.mockChunks.length / (avgSelectionTime / 1000);

      return {
        testCase: testCase.name,
        passed: issues.length === 0,
        actualMetrics: {
          criticalSetCoverage: lastResult.criticalSetCoverage,
          selectionTime: lastResult.selectionTime,
          totalTokens: lastResult.totalTokens,
          diversityScore: lastResult.diversityScore,
          chunksSelected: lastResult.selectedChunks.length
        },
        expectedMetrics: testCase.expectedMetrics,
        issues,
        performance: {
          avgSelectionTime,
          p95SelectionTime,
          throughputChunksPerSecond
        }
      };

    } catch (testError) {
      error(`[MMRTest] Test ${testCase.name} failed with error: ${testError instanceof Error ? testError.message : testError}`);
      
      return {
        testCase: testCase.name,
        passed: false,
        actualMetrics: {
          criticalSetCoverage: 0,
          selectionTime: 0,
          totalTokens: 0,
          diversityScore: 0,
          chunksSelected: 0
        },
        expectedMetrics: testCase.expectedMetrics,
        issues: [`Test execution failed: ${testError instanceof Error ? testError.message : testError}`],
        performance: {
          avgSelectionTime: 0,
          p95SelectionTime: 0,
          throughputChunksPerSecond: 0
        }
      };
    }
  }

  async runFullTestSuite(): Promise<{
    results: MMRTestResult[];
    summary: {
      totalTests: number;
      passed: number;
      failed: number;
      passRate: number;
      avgSelectionTime: number;
      avgThroughput: number;
    };
  }> {
    log('[MMRTest] Starting comprehensive MMR test suite');
    const testCases = this.getTestCases();
    const results: MMRTestResult[] = [];

    for (const testCase of testCases) {
      log(`[MMRTest] Running test: ${testCase.name} - ${testCase.description}`);
      const result = await this.runSingleTest(testCase);
      results.push(result);
      
      const status = result.passed ? '✅ PASS' : '❌ FAIL';
      log(`[MMRTest] ${status} - ${testCase.name} (${result.actualMetrics.chunksSelected}/${testCase.mockChunks.length} chunks, ${result.performance.avgSelectionTime.toFixed(2)}ms)`);
      
      if (!result.passed) {
        result.issues.forEach(issue => warn(`[MMRTest]   Issue: ${issue}`));
      }
    }

    // Calculate summary statistics
    const passed = results.filter(r => r.passed).length;
    const failed = results.length - passed;
    const passRate = (passed / results.length) * 100;
    
    const avgSelectionTime = results.reduce((sum, r) => sum + r.performance.avgSelectionTime, 0) / results.length;
    const avgThroughput = results.reduce((sum, r) => sum + r.performance.throughputChunksPerSecond, 0) / results.length;

    const summary = {
      totalTests: results.length,
      passed,
      failed,
      passRate,
      avgSelectionTime,
      avgThroughput
    };

    log(`[MMRTest] Test suite complete: ${passed}/${results.length} passed (${passRate.toFixed(1)}%)`);
    log(`[MMRTest] Average selection time: ${avgSelectionTime.toFixed(2)}ms`);
    log(`[MMRTest] Average throughput: ${avgThroughput.toFixed(0)} chunks/sec`);

    return { results, summary };
  }

  async runConfigurationTests(): Promise<void> {
    log('[MMRTest] Testing MMR configuration management');

    // Test preset application
    const presets = this.configManager.getAvailablePresets();
    for (const preset of presets) {
      try {
        const config = await this.configManager.applyPreset(preset);
        log(`[MMRTest] ✅ Applied preset: ${preset}`);
        
        // Test with preset
        const testSelector = new GuardedMMRSelector(config);
        const testChunks = this.createTestChunks(50);
        const result = await testSelector.selectOptimalChunks(testChunks, {
          task: 'Test query for preset validation',
          max_chunks: 20
        });
        
        if (result.selectedChunks.length > 0) {
          log(`[MMRTest] ✅ Preset ${preset} selection successful: ${result.selectedChunks.length} chunks`);
        } else {
          warn(`[MMRTest] ⚠️ Preset ${preset} returned no chunks`);
        }
        
      } catch (error) {
        warn(`[MMRTest] ❌ Failed to test preset ${preset}: ${error instanceof Error ? error.message : error}`);
      }
    }

    // Reset to defaults
    await this.configManager.resetToDefaults();
    log('[MMRTest] Configuration tests complete');
  }

  generateTestReport(results: MMRTestResult[]): string {
    const report = [
      '🧪 MMR Test Suite Report',
      '========================',
      '',
      `📊 Summary:`,
      `  Total Tests: ${results.length}`,
      `  Passed: ${results.filter(r => r.passed).length}`,
      `  Failed: ${results.filter(r => !r.passed).length}`,
      `  Pass Rate: ${((results.filter(r => r.passed).length / results.length) * 100).toFixed(1)}%`,
      '',
      `⚡ Performance Metrics:`,
      `  Avg Selection Time: ${(results.reduce((sum, r) => sum + r.performance.avgSelectionTime, 0) / results.length).toFixed(2)}ms`,
      `  P95 Selection Time: ${Math.max(...results.map(r => r.performance.p95SelectionTime)).toFixed(2)}ms`,
      `  Avg Throughput: ${(results.reduce((sum, r) => sum + r.performance.throughputChunksPerSecond, 0) / results.length).toFixed(0)} chunks/sec`,
      '',
      '📋 Detailed Results:',
      ''
    ];

    for (const result of results) {
      const status = result.passed ? '✅' : '❌';
      report.push(`${status} ${result.testCase}`);
      report.push(`   Critical Coverage: ${(result.actualMetrics.criticalSetCoverage * 100).toFixed(1)}% (min: ${(result.expectedMetrics.minCriticalSetCoverage * 100).toFixed(1)}%)`);
      report.push(`   Selection Time: ${result.actualMetrics.selectionTime.toFixed(2)}ms (max: ${result.expectedMetrics.maxSelectionTime}ms)`);
      report.push(`   Tokens: ${result.actualMetrics.totalTokens.toLocaleString()} (max: ${result.expectedMetrics.maxTokens.toLocaleString()})`);
      report.push(`   Diversity Score: ${result.actualMetrics.diversityScore.toFixed(3)} (min: ${result.expectedMetrics.minDiversityScore})`);
      report.push(`   Chunks Selected: ${result.actualMetrics.chunksSelected}`);
      
      if (result.issues.length > 0) {
        report.push(`   Issues:`);
        result.issues.forEach(issue => report.push(`     • ${issue}`));
      }
      
      report.push('');
    }

    return report.join('\n');
  }
}

// Performance benchmark specifically for MMR
export class MMRPerformanceBenchmark {
  private selector: GuardedMMRSelector;

  constructor() {
    this.selector = new GuardedMMRSelector();
  }

  async benchmarkScalability(): Promise<void> {
    log('[MMRBenchmark] Running MMR scalability benchmark');

    const testSizes = [10, 50, 100, 200, 500, 1000, 2000];
    const results = [];

    for (const size of testSizes) {
      const chunks = this.createBenchmarkChunks(size);
      const query = { task: 'Comprehensive analysis', max_chunks: Math.min(50, Math.floor(size / 4)) };

      const iterations = size > 500 ? 3 : 10;
      const times: number[] = [];

      for (let i = 0; i < iterations; i++) {
        const startTime = performance.now();
        const result = await this.selector.selectOptimalChunks(chunks, query);
        const endTime = performance.now();
        times.push(endTime - startTime);
      }

      const avgTime = times.reduce((sum, time) => sum + time, 0) / times.length;
      const throughput = size / (avgTime / 1000);

      results.push({ size, avgTime, throughput });
      log(`[MMRBenchmark] ${size} chunks: ${avgTime.toFixed(2)}ms (${throughput.toFixed(0)} chunks/sec)`);
    }

    // Check for performance degradation
    for (let i = 1; i < results.length; i++) {
      const prev = results[i - 1];
      const curr = results[i];
      const sizeRatio = curr.size / prev.size;
      const timeRatio = curr.avgTime / prev.avgTime;
      
      if (timeRatio > sizeRatio * 2) {
        warn(`[MMRBenchmark] Performance degradation detected: ${prev.size}→${curr.size} chunks, time ratio: ${timeRatio.toFixed(2)}x vs expected ${sizeRatio.toFixed(2)}x`);
      }
    }

    log('[MMRBenchmark] Scalability benchmark complete');
  }

  private createBenchmarkChunks(count: number): CodeChunk[] {
    // Create a more realistic distribution for benchmarking
    const chunks: CodeChunk[] = [];
    
    for (let i = 0; i < count; i++) {
      // More realistic embedding vectors
      const embedding = new Array(384).fill(0).map((_, idx) => {
        // Create some clustering structure
        const cluster = Math.floor(i / 50);
        return Math.sin(idx * cluster) * 0.5 + (Math.random() - 0.5) * 0.1;
      });
      
      chunks.push({
        chunk_id: `benchmark_${i}`,
        file_path: `src/module_${Math.floor(i / 20)}/file_${Math.floor(i / 5)}.ts`,
        symbol_name: `Symbol_${i}`,
        function_name: i % 3 === 0 ? `func_${i}` : undefined,
        chunk_type: ['function', 'class', 'method', 'documentation'][i % 4] as any,
        start_line: i * 15,
        end_line: i * 15 + 15,
        content: `// Benchmark content ${i}\n`.repeat(20 + (i % 30)), // Variable content length
        content_hash: `benchmark_hash_${i}`,
        embedding,
        relationships: {
          calls: Array.from({ length: Math.min(3, i) }, (_, idx) => `Symbol_${i - idx - 1}`),
          called_by: [],
          imports: [`import_${Math.floor(i / 10)}`],
          exports: [`export_${i}`],
          data_flow: []
        },
        git_metadata: {
          last_modified_commit: `commit_${i}`,
          commit_author: 'benchmark_author',
          commit_message: `Benchmark commit ${i}`,
          commit_date: new Date().toISOString(),
          file_history_length: 10,
          co_change_files: []
        },
        language_metadata: {
          language: 'typescript',
          complexity_score: 1 + (i % 10),
          dependencies: [],
          exports: []
        },
        usage_patterns: {
          access_frequency: Math.random(),
          task_contexts: []
        },
        last_modified: new Date().toISOString(),
        similarity_score: 0.5 + Math.random() * 0.5,
        relevance_score: 0.3 + Math.random() * 0.7
      });
    }
    
    return chunks;
  }
}