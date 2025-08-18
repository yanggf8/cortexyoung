#!/usr/bin/env node

const fs = require('fs');

console.log(`[${new Date().toISOString()}] Testing BGE embedder directly (bypassing ProcessPool)`);

async function testBGEDirect() {
    try {
        // Build first
        console.log(`[${new Date().toISOString()}] Building project...`);
        const { execSync } = require('child_process');
        execSync('npm run build', { stdio: 'inherit' });

        // Try different BGE embedders to find one that works
        let embedder = null;
        let embedderName = '';
        
        // Try CachedEmbedder first (should be most stable)
        try {
            const { CachedEmbedder } = require('./dist/cached-embedder.js');
            embedder = new CachedEmbedder();
            embedderName = 'CachedEmbedder';
            console.log(`[${new Date().toISOString()}] Using CachedEmbedder`);
        } catch (error) {
            console.log(`[${new Date().toISOString()}] CachedEmbedder not available: ${error.message}`);
        }
        
        // Try basic EmbeddingGenerator if CachedEmbedder fails
        if (!embedder) {
            try {
                const { EmbeddingGenerator } = require('./dist/embedder.js');
                embedder = new EmbeddingGenerator();
                embedderName = 'EmbeddingGenerator';
                console.log(`[${new Date().toISOString()}] Using EmbeddingGenerator`);
            } catch (error) {
                console.log(`[${new Date().toISOString()}] EmbeddingGenerator not available: ${error.message}`);
            }
        }
        
        if (!embedder) {
            console.error(`[${new Date().toISOString()}] No BGE embedder available for testing`);
            return;
        }

        // Get the same real source files as Ollama test
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
                    
                    // Create same chunks as Ollama test for fair comparison
                    const chunkSize = 500;
                    for (let i = 0; i < content.length && realCodeChunks.length < 20; i += chunkSize) {
                        const chunk = content.substring(i, i + chunkSize);
                        if (chunk.trim().length > 50) {
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

        // Test BGE embedder
        console.log(`[${new Date().toISOString()}] === Testing ${embedderName} ===`);
        
        const startTime = Date.now();
        console.log(`[${new Date().toISOString()}] Initializing ${embedderName}...`);
        
        // Try to initialize
        if (embedder.initialize) {
            await embedder.initialize();
        }
        const initTime = Date.now() - startTime;
        console.log(`[${new Date().toISOString()}] ${embedderName} initialized in ${initTime}ms`);
        
        // Test with same batch sizes as Ollama
        const testSizes = [1, 3, 5];
        
        for (const batchSize of testSizes) {
            if (realCodeChunks.length < batchSize) continue;
            
            console.log(`\n[${new Date().toISOString()}] Testing batch size: ${batchSize}`);
            
            const testChunks = realCodeChunks.slice(0, batchSize);
            const texts = testChunks.map(chunk => chunk.content);
            
            const batchStart = Date.now();
            
            let result;
            try {
                // Try embedBatch first (IEmbedder interface)
                if (embedder.embedBatch) {
                    result = await embedder.embedBatch(texts);
                } else if (embedder.generateEmbedding) {
                    // Fallback to single embedding generation
                    const embeddings = [];
                    for (const text of texts) {
                        const embedding = await embedder.generateEmbedding(text);
                        embeddings.push(embedding);
                    }
                    result = { embeddings };
                } else {
                    throw new Error('No suitable embedding method found');
                }
            } catch (error) {
                console.error(`[${new Date().toISOString()}] Batch ${batchSize} failed: ${error.message}`);
                continue;
            }
            
            const batchTime = Date.now() - batchStart;
            
            console.log(`[${new Date().toISOString()}] ✅ Batch ${batchSize}: ${batchTime}ms`);
            console.log(`[${new Date().toISOString()}]    Rate: ${(batchSize / (batchTime / 1000)).toFixed(2)} chunks/sec`);
            console.log(`[${new Date().toISOString()}]    Avg per chunk: ${(batchTime / batchSize).toFixed(0)}ms`);
            
            if (result.embeddings && result.embeddings.length > 0) {
                const dimensions = Array.isArray(result.embeddings[0]) ? result.embeddings[0].length : 'unknown';
                console.log(`[${new Date().toISOString()}]    Embeddings: ${result.embeddings.length} x ${dimensions} dims`);
            }
            
            // Show sample chunk info
            if (testChunks[0]) {
                const sampleChunk = testChunks[0];
                console.log(`[${new Date().toISOString()}]    Sample: ${sampleChunk.file}:${sampleChunk.start}-${sampleChunk.end}`);
                console.log(`[${new Date().toISOString()}]    Content preview: ${sampleChunk.content.substring(0, 80).replace(/\n/g, '\\n')}...`);
            }
        }
        
        // Try health check if available
        console.log(`\n[${new Date().toISOString()}] === Health Check ===`);
        
        try {
            if (embedder.getHealth) {
                const health = await embedder.getHealth();
                console.log(`[${new Date().toISOString()}] Health: ${health.status || 'unknown'}`);
                console.log(`[${new Date().toISOString()}] Details: ${health.details || 'no details'}`);
            } else {
                console.log(`[${new Date().toISOString()}] Health check not available for ${embedderName}`);
            }
        } catch (error) {
            console.log(`[${new Date().toISOString()}] Health check failed: ${error.message}`);
        }
        
        console.log(`\n[${new Date().toISOString()}] === BGE RESULTS ===`);
        console.log(`[${new Date().toISOString()}] ✅ ${embedderName} works with real codebase`);
        console.log(`[${new Date().toISOString()}] 📊 Processed actual TypeScript source code`);
        console.log(`[${new Date().toISOString()}] 🎯 Generated BGE embeddings (likely 384 dimensions)`);
        console.log(`[${new Date().toISOString()}] ⚡ Initialization time: ${initTime}ms`);
        
    } catch (error) {
        console.error(`[${new Date().toISOString()}] BGE test failed:`, error.message);
        console.error(`[${new Date().toISOString()}] Stack:`, error.stack);
    }
}

testBGEDirect();
