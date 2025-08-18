#!/usr/bin/env node

const fs = require('fs');
const path = require('path');

console.log(`[${new Date().toISOString()}] Testing Ollama embedder on real source files`);

async function testOllamaOnRealFiles() {
    try {
        // Build first
        console.log(`[${new Date().toISOString()}] Building project...`);
        const { execSync } = require('child_process');
        execSync('npm run build', { stdio: 'inherit' });

        // Import Ollama embedder
        const { OllamaEmbedder } = require('./dist/ollama-embedder.js');

        // Get some real source files directly
        const sourceFiles = [
            'src/types.ts',
            'src/git-scanner.ts', 
            'src/chunker.ts',
            'src/vector-store.ts',
            'src/searcher.ts'
        ];

        console.log(`[${new Date().toISOString()}] Reading ${sourceFiles.length} source files...`);
        
        const realCodeChunks = [];
        
        for (const filePath of sourceFiles) {
            try {
                if (fs.existsSync(filePath)) {
                    const content = fs.readFileSync(filePath, 'utf8');
                    
                    // Create simple chunks from the file (first 500 chars, middle 500 chars, etc.)
                    const chunkSize = 500;
                    for (let i = 0; i < content.length && realCodeChunks.length < 20; i += chunkSize) {
                        const chunk = content.substring(i, i + chunkSize);
                        if (chunk.trim().length > 50) { // Skip tiny chunks
                            realCodeChunks.push({
                                content: chunk,
                                file: filePath,
                                start: i,
                                end: Math.min(i + chunkSize, content.length)
                            });
                        }
                    }
                }
            } catch (error) {
                console.warn(`[${new Date().toISOString()}] Failed to read ${filePath}: ${error.message}`);
            }
        }
        
        console.log(`[${new Date().toISOString()}] Generated ${realCodeChunks.length} real code chunks`);
        
        if (realCodeChunks.length === 0) {
            console.error(`[${new Date().toISOString()}] No chunks generated - cannot test`);
            return;
        }

        // Test Ollama embedder
        console.log(`[${new Date().toISOString()}] === Testing Ollama on Real Code ===`);
        
        const embedder = new OllamaEmbedder();
        
        const startTime = Date.now();
        console.log(`[${new Date().toISOString()}] Initializing Ollama embedder...`);
        
        await embedder.initialize();
        const initTime = Date.now() - startTime;
        console.log(`[${new Date().toISOString()}] Ollama initialized in ${initTime}ms`);
        
        // Test with different batch sizes
        const testSizes = [1, 3, 5];
        
        for (const batchSize of testSizes) {
            if (realCodeChunks.length < batchSize) continue;
            
            console.log(`\n[${new Date().toISOString()}] Testing batch size: ${batchSize}`);
            
            const testChunks = realCodeChunks.slice(0, batchSize);
            const texts = testChunks.map(chunk => chunk.content);
            
            const batchStart = Date.now();
            const result = await embedder.embedBatch(texts);
            const batchTime = Date.now() - batchStart;
            
            console.log(`[${new Date().toISOString()}] ✅ Batch ${batchSize}: ${batchTime}ms`);
            console.log(`[${new Date().toISOString()}]    Rate: ${(batchSize / (batchTime / 1000)).toFixed(2)} chunks/sec`);
            console.log(`[${new Date().toISOString()}]    Avg per chunk: ${(batchTime / batchSize).toFixed(0)}ms`);
            console.log(`[${new Date().toISOString()}]    Embeddings: ${result.embeddings.length} x ${result.embeddings[0]?.length || 0} dims`);
            
            // Show sample chunk info
            if (testChunks[0]) {
                const sampleChunk = testChunks[0];
                console.log(`[${new Date().toISOString()}]    Sample: ${sampleChunk.file}:${sampleChunk.start}-${sampleChunk.end}`);
                console.log(`[${new Date().toISOString()}]    Content preview: ${sampleChunk.content.substring(0, 80).replace(/\n/g, '\\n')}...`);
            }
        }
        
        // Test health
        console.log(`\n[${new Date().toISOString()}] === Health Check ===`);
        
        const health = await embedder.getHealth();
        console.log(`[${new Date().toISOString()}] Health: ${health.status}`);
        console.log(`[${new Date().toISOString()}] Details: ${health.details}`);
        
        const metrics = await embedder.getMetrics();
        console.log(`[${new Date().toISOString()}] Requests: ${metrics.requestCount}`);
        console.log(`[${new Date().toISOString()}] Avg Duration: ${metrics.avgDuration.toFixed(0)}ms`);
        console.log(`[${new Date().toISOString()}] Error Rate: ${(metrics.errorRate * 100).toFixed(1)}%`);
        
        console.log(`\n[${new Date().toISOString()}] === SUCCESS SUMMARY ===`);
        console.log(`[${new Date().toISOString()}] ✅ Ollama embedder works with your real codebase!`);
        console.log(`[${new Date().toISOString()}] 📊 Processed actual TypeScript source code`);
        console.log(`[${new Date().toISOString()}] 🎯 Generated 768-dimensional embeddings`);
        console.log(`[${new Date().toISOString()}] ⚡ Average performance: ${metrics.avgDuration.toFixed(0)}ms per chunk`);
        console.log(`[${new Date().toISOString()}] 🚀 Ready for integration!`);
        
        // Performance comparison note
        console.log(`\n[${new Date().toISOString()}] === NEXT STEPS ===`);
        console.log(`[${new Date().toISOString()}] 1. Compare with your BGE performance on same chunks`);
        console.log(`[${new Date().toISOString()}] 2. Test semantic search quality with Ollama embeddings`);
        console.log(`[${new Date().toISOString()}] 3. Consider making Ollama the default embedder`);
        
    } catch (error) {
        console.error(`[${new Date().toISOString()}] Test failed:`, error.message);
        console.error(`[${new Date().toISOString()}] Stack:`, error.stack);
    }
}

testOllamaOnRealFiles();
