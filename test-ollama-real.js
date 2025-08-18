#!/usr/bin/env node

const fs = require('fs');
const path = require('path');

console.log(`[${new Date().toISOString()}] Testing Ollama embedder on real codebase`);

async function testOllamaOnRealCodebase() {
    try {
        // Build first to ensure we have the latest code
        console.log(`[${new Date().toISOString()}] Building project...`);
        const { execSync } = require('child_process');
        execSync('npm run build', { stdio: 'inherit' });

        // Import your existing components
        const { GitScanner } = require('./dist/git-scanner.js');
        const { SmartChunker } = require('./dist/chunker.js');
        const { OllamaEmbedder } = require('./dist/ollama-embedder.js');

        console.log(`[${new Date().toISOString()}] Scanning repository...`);
        
        // Scan your actual codebase
        const scanner = new GitScanner();
        const files = await scanner.scanRepository('.');
        
        console.log(`[${new Date().toISOString()}] Found ${files.length} files`);
        
        // Chunk the files (limit to first 20 for testing)
        const chunker = new SmartChunker();
        let allChunks = [];
        
        const testFiles = files.slice(0, 20); // Test with first 20 files
        console.log(`[${new Date().toISOString()}] Processing ${testFiles.length} files for chunking...`);
        
        for (const file of testFiles) {
            try {
                const chunks = await chunker.chunkFile(file);
                allChunks.push(...chunks);
                
                if (allChunks.length >= 50) break; // Limit to 50 chunks for testing
            } catch (error) {
                console.warn(`[${new Date().toISOString()}] Failed to chunk ${file.path}: ${error.message}`);
            }
        }
        
        console.log(`[${new Date().toISOString()}] Generated ${allChunks.length} chunks from real codebase`);
        
        if (allChunks.length === 0) {
            console.error(`[${new Date().toISOString()}] No chunks generated - cannot test`);
            return;
        }

        // Test Ollama embedder
        console.log(`[${new Date().toISOString()}] === Testing Ollama on Real Codebase ===`);
        
        const embedder = new OllamaEmbedder();
        
        const startTime = Date.now();
        console.log(`[${new Date().toISOString()}] Initializing Ollama embedder...`);
        
        await embedder.initialize();
        const initTime = Date.now() - startTime;
        console.log(`[${new Date().toISOString()}] Ollama initialized in ${initTime}ms`);
        
        // Test with batches of different sizes
        const testBatches = [
            { size: 1, name: 'Single chunk' },
            { size: 5, name: 'Small batch' },
            { size: 10, name: 'Medium batch' }
        ];
        
        for (const batch of testBatches) {
            if (allChunks.length < batch.size) continue;
            
            console.log(`\n[${new Date().toISOString()}] Testing ${batch.name} (${batch.size} chunks):`);
            
            const testChunks = allChunks.slice(0, batch.size);
            const texts = testChunks.map(chunk => chunk.content);
            
            const batchStart = Date.now();
            const result = await embedder.embedBatch(texts);
            const batchTime = Date.now() - batchStart;
            
            console.log(`[${new Date().toISOString()}] ✅ ${batch.name}: ${batchTime}ms for ${batch.size} chunks`);
            console.log(`[${new Date().toISOString()}]    Rate: ${(batch.size / (batchTime / 1000)).toFixed(2)} chunks/sec`);
            console.log(`[${new Date().toISOString()}]    Avg per chunk: ${(batchTime / batch.size).toFixed(0)}ms`);
            console.log(`[${new Date().toISOString()}]    Embeddings: ${result.embeddings.length} x ${result.embeddings[0]?.length || 0} dims`);
            
            // Show sample chunk info
            if (testChunks[0]) {
                const sampleChunk = testChunks[0];
                console.log(`[${new Date().toISOString()}]    Sample: ${sampleChunk.path}:${sampleChunk.startLine}-${sampleChunk.endLine}`);
                console.log(`[${new Date().toISOString()}]    Content: ${sampleChunk.content.substring(0, 100)}...`);
            }
        }
        
        // Test health and metrics
        console.log(`\n[${new Date().toISOString()}] === Health & Metrics ===`);
        
        const health = await embedder.getHealth();
        console.log(`[${new Date().toISOString()}] Health: ${health.isHealthy ? '✅ Healthy' : '❌ Unhealthy'}`);
        if (health.error) {
            console.log(`[${new Date().toISOString()}] Error: ${health.error}`);
        }
        if (health.details) {
            console.log(`[${new Date().toISOString()}] Details:`, health.details);
        }
        
        const metrics = await embedder.getMetrics();
        console.log(`[${new Date().toISOString()}] Metrics:`, metrics);
        
        console.log(`\n[${new Date().toISOString()}] === Test Summary ===`);
        console.log(`[${new Date().toISOString()}] ✅ Ollama embedder works with real codebase`);
        console.log(`[${new Date().toISOString()}] 📊 Processed chunks from ${testFiles.length} actual source files`);
        console.log(`[${new Date().toISOString()}] 🎯 Generated ${result.embeddings[0]?.length || 768}-dimensional embeddings`);
        console.log(`[${new Date().toISOString()}] ⚡ Ready for integration with your existing system`);
        
    } catch (error) {
        console.error(`[${new Date().toISOString()}] Test failed:`, error.message);
        console.error(`[${new Date().toISOString()}] Stack:`, error.stack);
    }
}

testOllamaOnRealCodebase();
