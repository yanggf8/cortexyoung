#!/usr/bin/env node

import { PerformanceBenchmark } from './performance-benchmark';
import * as path from 'path';

interface CliOptions {
  repositoryPath?: string;
  output?: string;
  category?: 'startup' | 'search' | 'storage' | 'all';
  verbose?: boolean;
  iterations?: number;
}

class BenchmarkCli {
  private options: CliOptions;

  constructor(options: CliOptions = {}) {
    this.options = {
      repositoryPath: process.cwd(),
      category: 'all',
      verbose: false,
      iterations: 1,
      ...options
    };
  }

  private parseArgs(): CliOptions {
    const args = process.argv.slice(2);
    const options: CliOptions = {};
    
    for (let i = 0; i < args.length; i++) {
      const arg = args[i];
      
      switch (arg) {
        case '--repo':
        case '-r':
          options.repositoryPath = args[++i];
          break;
        case '--output':
        case '-o':
          options.output = args[++i];
          break;
        case '--category':
        case '-c':
          options.category = args[++i] as 'startup' | 'search' | 'storage' | 'all';
          break;
        case '--verbose':
        case '-v':
          options.verbose = true;
          break;
        case '--iterations':
        case '-i':
          options.iterations = parseInt(args[++i]) || 1;
          break;
        case '--help':
        case '-h':
          this.showHelp();
          process.exit(0);
          break;
        default:
          if (!options.repositoryPath && !arg.startsWith('-')) {
            options.repositoryPath = arg;
          }
          break;
      }
    }
    
    return options;
  }

  private showHelp(): void {
    console.log(`
🏁 Cortex Performance Benchmark CLI

Usage:
  npm run benchmark [options] [repository-path]
  
Options:
  -r, --repo <path>        Repository path to benchmark (default: current directory)
  -c, --category <type>    Benchmark category: startup|search|storage|all (default: all)
  -o, --output <file>      Output file for JSON report (default: auto-generated)
  -i, --iterations <num>   Number of iterations to run (default: 1)
  -v, --verbose            Enable verbose output
  -h, --help               Show this help message

Categories:
  startup    - Cold start, warm start, cache loading performance
  search     - Query performance with various complexity levels
  storage    - Storage operations, sync, consistency checks
  all        - Run complete benchmark suite

Examples:
  npm run benchmark                           # Run all benchmarks on current repo
  npm run benchmark --category startup       # Only test startup performance  
  npm run benchmark --repo /path/to/project  # Test specific repository
  npm run benchmark --iterations 3 --verbose # Run 3 iterations with detailed output
  npm run benchmark --output my-report.json  # Save to specific file

Performance Targets:
  🎯 Cold start:     < 3 minutes (first run with model download)
  🎯 Warm start:     < 30 seconds (subsequent runs with cache)
  🎯 Cache loading:  < 5 seconds (pure cache loading)
  🎯 Search queries: < 500ms (semantic search with relationships)
  🎯 Memory usage:   < 1GB (peak during indexing)
`);
  }

  private async runCategoryBenchmarks(
    benchmark: PerformanceBenchmark, 
    category: string
  ): Promise<void> {
    switch (category) {
      case 'startup':
        console.log('🚀 Running Startup Benchmarks Only');
        await benchmark.runStartupBenchmarks();
        break;
      case 'search':
        console.log('🔍 Running Search Benchmarks Only');
        await benchmark.runSearchBenchmarks();
        break;
      case 'storage':
        console.log('💾 Running Storage Benchmarks Only');
        await benchmark.runStorageBenchmarks();
        break;
      case 'all':
      default:
        console.log('🏁 Running Full Benchmark Suite');
        await benchmark.runFullBenchmarkSuite();
        break;
    }
  }

  private formatDuration(ms: number): string {
    if (ms < 1000) return `${ms.toFixed(0)}ms`;
    if (ms < 60000) return `${(ms / 1000).toFixed(1)}s`;
    return `${(ms / 60000).toFixed(1)}m`;
  }

  private printSummary(report: any): void {
    console.log('\n📊 Performance Summary');
    console.log('=====================');
    
    // System info
    console.log(`\n🖥️  System: ${report.systemInfo.platform} ${report.systemInfo.arch}`);
    console.log(`🔧 Node.js: ${report.systemInfo.nodeVersion}`);
    console.log(`📁 Repository: ${report.repositoryInfo.fileCount} files`);
    
    // Key metrics
    const startupBenchmarks = report.benchmarks.startup;
    const searchBenchmarks = report.benchmarks.search;
    
    if (startupBenchmarks.length > 0) {
      console.log('\n⚡ Startup Performance:');
      startupBenchmarks.forEach((b: any) => {
        const icon = b.success ? '✅' : '❌';
        console.log(`   ${icon} ${b.name}: ${this.formatDuration(b.duration)}`);
      });
    }
    
    if (searchBenchmarks.length > 0) {
      console.log('\n🔍 Search Performance:');
      const avgSearchTime = searchBenchmarks.reduce((sum: number, b: any) => sum + b.duration, 0) / searchBenchmarks.length;
      console.log(`   📊 Average query time: ${this.formatDuration(avgSearchTime)}`);
      
      const fastestSearch = Math.min(...searchBenchmarks.map((b: any) => b.duration));
      const slowestSearch = Math.max(...searchBenchmarks.map((b: any) => b.duration));
      console.log(`   ⚡ Fastest query: ${this.formatDuration(fastestSearch)}`);
      console.log(`   🐌 Slowest query: ${this.formatDuration(slowestSearch)}`);
    }
    
    // Overall stats
    console.log(`\n📈 Overall Stats:`);
    console.log(`   ✅ Success Rate: ${report.summary.successRate.toFixed(1)}%`);
    console.log(`   🧠 Peak Memory: ${report.summary.memoryPeakMB.toFixed(1)}MB`);
    console.log(`   ⏱️  Total Time: ${this.formatDuration(report.summary.totalDuration)}`);
    
    // Recommendations
    if (report.summary.recommendedActions.length > 0) {
      console.log('\n💡 Recommendations:');
      report.summary.recommendedActions.forEach((action: string) => {
        console.log(`   • ${action}`);
      });
    }
  }

  async run(): Promise<void> {
    const options = { ...this.options, ...this.parseArgs() };
    
    console.log('🏁 Cortex Performance Benchmark');
    console.log('===============================');
    console.log(`📁 Repository: ${options.repositoryPath}`);
    console.log(`📊 Category: ${options.category}`);
    console.log(`🔄 Iterations: ${options.iterations}`);
    console.log('');
    
    const benchmark = new PerformanceBenchmark(options.repositoryPath!);
    let cumulativeReport: any = null;
    
    try {
      // Run benchmark iterations
      for (let iteration = 1; iteration <= options.iterations!; iteration++) {
        if (options.iterations! > 1) {
          console.log(`\n🔄 Iteration ${iteration}/${options.iterations}`);
          console.log('─'.repeat(30));
        }
        
        if (options.category === 'all') {
          const report = await benchmark.runFullBenchmarkSuite();
          cumulativeReport = report;
        } else {
          await this.runCategoryBenchmarks(benchmark, options.category!);
          cumulativeReport = await benchmark.generatePerformanceReport();
        }
        
        if (options.verbose) {
          this.printSummary(cumulativeReport);
        }
      }
      
      // Save report
      if (cumulativeReport) {
        const reportPath = await benchmark.saveReport(cumulativeReport, options.output);
        
        console.log('\n🎉 Benchmark Complete!');
        console.log(`📄 Report saved: ${path.relative(process.cwd(), reportPath)}`);
        
        if (!options.verbose) {
          this.printSummary(cumulativeReport);
        }
      }
      
    } catch (error) {
      console.error('\n❌ Benchmark failed:', error instanceof Error ? error.message : error);
      process.exit(1);
    }
  }
}

// Run CLI if this file is executed directly
if (require.main === module) {
  const cli = new BenchmarkCli();
  cli.run().catch(console.error);
}

export { BenchmarkCli };