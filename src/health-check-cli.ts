#!/usr/bin/env ts-node

import { IndexHealthChecker, HealthReport } from './index-health-checker';
import { PersistentVectorStore } from './persistent-vector-store';

async function runHealthCheck(): Promise<void> {
  const repoPath = process.argv[2] || process.cwd();
  const command = process.argv[3] || 'check';

  console.log(`🩺 Running health check on repository: ${repoPath}`);
  console.log('');

  try {
    const vectorStore = new PersistentVectorStore(repoPath);
    const healthChecker = new IndexHealthChecker(repoPath, vectorStore);

    if (command === 'check') {
      const report = await healthChecker.performHealthCheck();
      printHealthReport(report);
    } else if (command === 'should-rebuild') {
      const result = await healthChecker.shouldRebuild();
      console.log(`Should rebuild: ${result.shouldRebuild}`);
      console.log(`Reason: ${result.reason}`);
      console.log(`Recommended mode: ${result.mode}`);
      
      // Exit with appropriate code
      process.exit(result.shouldRebuild ? 1 : 0);
    } else {
      console.error(`Unknown command: ${command}`);
      console.log('Available commands: check, should-rebuild');
      process.exit(1);
    }

  } catch (error) {
    console.error('❌ Health check failed:', error instanceof Error ? error.message : error);
    process.exit(1);
  }
}

function printHealthReport(report: HealthReport): void {
  // Print overall status
  const statusEmoji = {
    'healthy': '✅',
    'degraded': '⚠️',
    'critical': '❌'
  };

  console.log(`${statusEmoji[report.overall]} Overall Health: ${report.overall.toUpperCase()}`);
  console.log('');

  // Print stats
  console.log('📊 Index Statistics:');
  console.log(`  Total chunks: ${report.stats.totalChunks}`);
  console.log(`  Total files: ${report.stats.totalFiles}`);
  console.log(`  Last indexed: ${report.stats.lastIndexed ? report.stats.lastIndexed.toLocaleString() : 'Never'}`);
  console.log(`  Index age: ${Math.floor(report.stats.indexAge / (1000 * 60 * 60 * 24))} days`);
  console.log(`  Embedding model: ${report.stats.embeddingModel}`);
  console.log(`  Schema version: ${report.stats.schemaVersion}`);
  console.log('');

  // Print issues
  if (report.issues.length > 0) {
    console.log('🔍 Issues Found:');
    report.issues.forEach((issue, index) => {
      const severityEmoji = {
        'critical': '🚨',
        'warning': '⚠️',
        'info': 'ℹ️'
      };
      
      console.log(`  ${index + 1}. ${severityEmoji[issue.severity]} [${issue.severity.toUpperCase()}] ${issue.message}`);
      if (issue.details) {
        console.log(`     Details: ${JSON.stringify(issue.details, null, 6).replace(/\n/g, '\n     ')}`);
      }
      console.log(`     Suggested action: ${issue.suggestedAction.replace('_', ' ')}`);
      console.log('');
    });
  } else {
    console.log('✅ No issues found!');
    console.log('');
  }

  // Print recommendations
  if (report.recommendations.length > 0) {
    console.log('💡 Recommendations:');
    report.recommendations.forEach(rec => {
      console.log(`  • ${rec}`);
    });
    console.log('');
  }

  // Exit with appropriate code
  const exitCode = report.overall === 'critical' ? 2 : report.overall === 'degraded' ? 1 : 0;
  process.exit(exitCode);
}

// Handle CLI execution
if (require.main === module) {
  runHealthCheck().catch(error => {
    console.error('❌ Health check failed:', error);
    process.exit(1);
  });
}