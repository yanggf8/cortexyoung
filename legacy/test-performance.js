import { testOptimizedDetection } from './dist/optimized-pattern-detector.js';
import { testIntegratedDetection } from './dist/integrated-pattern-detector.js';

async function benchmarkPerformance() {
    console.log('🏁 Performance Benchmark: Optimized vs Standard Detection\n');
    
    const projectPath = '/tmp/ground-truth-testing/express-jwt-passport-local-mongoose-winston';
    
    try {
        // Test 1: Standard integrated detection
        console.log('=== STANDARD DETECTION ===');
        const startStandard = Date.now();
        const standardResult = await testIntegratedDetection(projectPath);
        const standardTime = Date.now() - startStandard;
        
        console.log('\n=== OPTIMIZED DETECTION ===');
        const startOptimized = Date.now();
        const optimizedResults = await testOptimizedDetection(projectPath);
        const optimizedTime = Date.now() - startOptimized;
        
        console.log('\n=== PERFORMANCE COMPARISON ===');
        console.log(`Standard Detection: ${standardTime}ms`);
        console.log(`Optimized Detection: ${optimizedTime}ms`);
        
        const improvement = ((standardTime - optimizedTime) / standardTime * 100);
        console.log(`Performance Improvement: ${improvement > 0 ? '+' : ''}${Math.round(improvement)}%`);
        
        if (optimizedResults.metrics) {
            console.log(`\nOptimized Metrics:`);
            console.log(`  Cache Hits: ${optimizedResults.metrics.cacheHits}`);
            console.log(`  Files/Second: ${Math.round(optimizedResults.metrics.filesProcessed / (optimizedResults.metrics.totalTime / 1000))}`);
        }
        
        return {
            standardTime,
            optimizedTime,
            improvement: Math.round(improvement),
            results: { standard: standardResult, optimized: optimizedResults }
        };
        
    } catch (error) {
        console.error('❌ Benchmark failed:', error);
        throw error;
    }
}

benchmarkPerformance();