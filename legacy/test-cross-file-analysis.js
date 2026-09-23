#!/usr/bin/env node

/**
 * Test Cross-File Analysis System
 * Validates the new cross-file pattern detection capabilities
 */

import { CrossFileEnhancedProjectDetector } from './dist/core/enhanced-project-detector.js';
import { performance } from 'perf_hooks';

async function testCrossFileAnalysis() {
  console.log('🧪 Testing Cross-File Analysis System\n');

  const detector = new CrossFileEnhancedProjectDetector(process.cwd());

  try {
    // Test 1: Basic cross-file detection
    console.log('🔍 Test 1: Basic Cross-File Detection');
    const startTime = performance.now();
    
    const analysis = await detector.detectComprehensiveContext();
    
    const endTime = performance.now();
    const executionTime = Math.round(endTime - startTime);

    console.log(`✅ Analysis completed in ${executionTime}ms`);
    console.log(`📊 Files analyzed: ${analysis.crossFileAnalysis.metadata.filesAnalyzed}`);
    console.log(`🏛️ Architecture: ${analysis.architecture}`);
    console.log(`🎯 Overall confidence: ${analysis.enhancedConfidence.overallAccuracy}%`);
    console.log(`📈 Cross-file accuracy: ${analysis.enhancedConfidence.crossFileAccuracy}%\n`);

    // Test 2: Dependency graph statistics
    console.log('🔍 Test 2: Dependency Graph Analysis');
    const graph = analysis.crossFileAnalysis.dependencyGraph;
    const totalImports = Array.from(graph.imports.values()).reduce((sum, imports) => sum + imports.length, 0);
    const totalExports = Array.from(graph.exports.values()).reduce((sum, exports) => sum + exports.length, 0);
    const totalSymbols = Array.from(graph.symbols.values()).reduce((sum, symbols) => sum + symbols.length, 0);

    console.log(`📁 Total files: ${graph.files.size}`);
    console.log(`📥 Total imports: ${totalImports}`);
    console.log(`📤 Total exports: ${totalExports}`);
    console.log(`🎯 Total symbols: ${totalSymbols}`);
    console.log(`🔗 Dependency edges: ${graph.dependencies.size}`);
    console.log(`🔄 Reverse dependencies: ${graph.dependents.size}\n`);

    // Test 3: Pattern detection results
    console.log('🔍 Test 3: Pattern Detection Results');
    const patterns = analysis.crossFileAnalysis.patterns;
    console.log(`🧩 Total patterns detected: ${patterns.length}`);

    const patternTypes = patterns.reduce((acc, pattern) => {
      acc[pattern.type] = (acc[pattern.type] || 0) + 1;
      return acc;
    }, {});

    console.log('📊 Pattern breakdown:');
    for (const [type, count] of Object.entries(patternTypes)) {
      console.log(`   ${type}: ${count}`);
    }
    console.log();

    // Test 4: Execution flow analysis
    console.log('🔍 Test 4: Execution Flow Analysis');
    const flows = analysis.crossFileAnalysis.executionFlows;
    console.log(`🛤️ Total execution paths: ${flows.length}`);
    
    if (flows.length > 0) {
      const avgPathLength = flows.reduce((sum, flow) => sum + flow.callChain.length, 0) / flows.length;
      const maxPathLength = Math.max(...flows.map(flow => flow.callChain.length));
      console.log(`📏 Average path length: ${Math.round(avgPathLength * 100) / 100}`);
      console.log(`📏 Maximum path length: ${maxPathLength}`);
    }
    console.log();

    // Test 5: Implementation pattern comparison
    console.log('🔍 Test 5: Implementation Pattern Comparison');
    console.log('Authentication patterns:');
    console.log(`   Single-file confidence: ${analysis.enhancedConfidence.singleFileAccuracy}%`);
    console.log(`   Cross-file confidence: ${analysis.enhancedConfidence.crossFileAccuracy}%`);
    console.log(`   User property: ${analysis.implementationPatterns.authentication.userProperty}`);
    console.log(`   Middleware pattern: ${analysis.implementationPatterns.authentication.middlewarePattern}`);
    console.log();

    console.log('API Response patterns:');
    console.log(`   Success format: ${analysis.implementationPatterns.apiResponses.successFormat}`);
    console.log(`   Error format: ${analysis.implementationPatterns.apiResponses.errorFormat}`);
    console.log(`   Wrapper pattern: ${analysis.implementationPatterns.apiResponses.wrapperPattern}`);
    console.log();

    // Test 6: Architectural insights
    console.log('🔍 Test 6: Architectural Insights');
    const insights = await detector.getArchitecturalInsights();
    console.log(`🏛️ Architecture type: ${insights.architecture}`);
    console.log(`📚 Layer count: ${insights.layerCount}`);
    console.log(`🧩 Component count: ${insights.componentCount}`);
    console.log(`⚡ Complexity: ${insights.complexity}`);
    console.log('💡 Recommendations:');
    insights.recommendations.forEach(rec => {
      console.log(`   • ${rec}`);
    });
    console.log();

    // Test 7: Performance comparison
    console.log('🔍 Test 7: Performance Comparison');
    console.log(`⚡ Total analysis time: ${executionTime}ms`);
    console.log(`⚡ Cross-file analysis time: ${analysis.crossFileAnalysis.metadata.executionTimeMs}ms`);
    console.log(`⚡ Files per second: ${Math.round((analysis.crossFileAnalysis.metadata.filesAnalyzed / analysis.crossFileAnalysis.metadata.executionTimeMs) * 1000)}`);
    console.log();

    // Summary
    console.log('📋 Summary');
    console.log(`✅ Cross-file analysis: ${analysis.enhancedConfidence.crossFileAccuracy >= 60 ? 'SUCCESS' : 'NEEDS_IMPROVEMENT'}`);
    console.log(`✅ Pattern detection: ${patterns.length > 0 ? 'SUCCESS' : 'NO_PATTERNS'}`);
    console.log(`✅ Execution flow tracing: ${flows.length > 0 ? 'SUCCESS' : 'NO_FLOWS'}`);
    console.log(`✅ Performance: ${executionTime < 10000 ? 'GOOD' : 'NEEDS_OPTIMIZATION'}`);
    
    const overallSuccess = (
      analysis.enhancedConfidence.crossFileAccuracy >= 60 &&
      patterns.length > 0 &&
      flows.length > 0 &&
      executionTime < 10000
    );
    
    console.log(`\n🎯 Overall Result: ${overallSuccess ? '✅ SUCCESS' : '❌ NEEDS_IMPROVEMENT'}`);

    return overallSuccess;

  } catch (error) {
    console.error('❌ Cross-file analysis failed:', error);
    return false;
  }
}

async function testSpecificPatterns() {
  console.log('\n🧪 Testing Specific Pattern Detection\n');
  
  const detector = new CrossFileEnhancedProjectDetector(process.cwd());

  try {
    // Test authentication patterns
    console.log('🔐 Testing Authentication Pattern Detection');
    const authStart = performance.now();
    const authPatterns = await detector.detectSpecificPatterns(['authentication']);
    const authEnd = performance.now();
    
    console.log(`✅ Authentication analysis: ${Math.round(authEnd - authStart)}ms`);
    console.log(`📊 Confidence: ${authPatterns.authentication?.confidence || 0}%`);
    console.log(`🎯 User property: ${authPatterns.authentication?.userProperty || 'Unknown'}`);
    console.log();

    // Test API response patterns
    console.log('🌐 Testing API Response Pattern Detection');
    const apiStart = performance.now();
    const apiPatterns = await detector.detectSpecificPatterns(['apiResponses']);
    const apiEnd = performance.now();
    
    console.log(`✅ API response analysis: ${Math.round(apiEnd - apiStart)}ms`);
    console.log(`📊 Confidence: ${apiPatterns.apiResponses?.confidence || 0}%`);
    console.log(`🎯 Success format: ${apiPatterns.apiResponses?.successFormat || 'Unknown'}`);
    console.log();

    return true;

  } catch (error) {
    console.error('❌ Specific pattern testing failed:', error);
    return false;
  }
}

async function main() {
  console.log('🚀 ClaudeCat Cross-File Analysis Test Suite');
  console.log('============================================\n');

  const results = [];

  // Run comprehensive test
  results.push(await testCrossFileAnalysis());

  // Run specific pattern tests
  results.push(await testSpecificPatterns());

  const overallSuccess = results.every(result => result === true);

  console.log('\n============================================');
  console.log(`🏁 Test Suite Complete: ${overallSuccess ? '✅ ALL TESTS PASSED' : '❌ SOME TESTS FAILED'}`);
  console.log('============================================');

  process.exit(overallSuccess ? 0 : 1);
}

// Run if called directly
if (import.meta.url === `file://${process.argv[1]}`) {
  main().catch(error => {
    console.error('❌ Test suite failed:', error);
    process.exit(1);
  });
}

export { testCrossFileAnalysis, testSpecificPatterns };