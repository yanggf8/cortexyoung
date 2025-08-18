#!/usr/bin/env node

// Test chunks (same as Ollama test)
const testChunks = [
    "function calculateDistance(x1, y1, x2, y2) { return Math.sqrt((x2-x1)**2 + (y2-y1)**2); }",
    "class UserManager { constructor() { this.users = new Map(); } addUser(user) { this.users.set(user.id, user); } }",
    "const express = require('express'); const app = express(); app.get('/api/users', (req, res) => { res.json(users); });",
    "async function fetchUserData(userId) { const response = await fetch(`/api/users/${userId}`); return response.json(); }",
    "interface DatabaseConfig { host: string; port: number; database: string; credentials: AuthCredentials; }",
    "export class ApiClient { private baseUrl: string; constructor(baseUrl: string) { this.baseUrl = baseUrl; } }",
    "function debounce(func, wait) { let timeout; return function executedFunction(...args) { clearTimeout(timeout); timeout = setTimeout(() => func.apply(this, args), wait); }; }",
    "const validateEmail = (email) => /^[^\\s@]+@[^\\s@]+\\.[^\\s@]+$/.test(email);",
    "class EventEmitter { constructor() { this.events = {}; } on(event, listener) { if (!this.events[event]) this.events[event] = []; this.events[event].push(listener); } }",
    "function quickSort(arr) { if (arr.length <= 1) return arr; const pivot = arr[Math.floor(arr.length / 2)]; return [...quickSort(left), pivot, ...quickSort(right)]; }"
];

console.log(`[${new Date().toISOString()}] Starting BGE embedding performance test`);
console.log(`[${new Date().toISOString()}] Test chunks: ${testChunks.length}`);

async function runBGETest() {
    try {
        // Import your embedder
        const { ProcessPoolEmbedder } = require('./dist/process-pool-embedder.js');
        
        console.log(`\n[${new Date().toISOString()}] === Testing BGE-small-en-v1.5 ===`);
        
        const startTime = Date.now();
        const embedder = new ProcessPoolEmbedder();
        
        // Initialize embedder
        console.log(`[${new Date().toISOString()}] Initializing BGE embedder...`);
        const initStart = Date.now();
        await embedder.initialize();
        const initTime = Date.now() - initStart;
        console.log(`[${new Date().toISOString()}] BGE initialization took ${initTime}ms`);
        
        // Test individual chunks to match Ollama's approach
        let processedChunks = 0;
        let firstEmbeddingTime = 0;
        let totalDimensions = 0;
        
        for (const chunk of testChunks) {
            const chunkStartTime = Date.now();
            
            // Use embedBatch with single chunk
            const result = await embedder.embedBatch([chunk]);
            const chunkTime = Date.now() - chunkStartTime;
            
            processedChunks++;
            
            if (processedChunks === 1) {
                firstEmbeddingTime = chunkTime;
                if (result.embeddings && result.embeddings.length > 0) {
                    totalDimensions = result.embeddings[0].length;
                    console.log(`[${new Date().toISOString()}] First chunk processed in ${chunkTime}ms`);
                    console.log(`[${new Date().toISOString()}] Embedding dimensions: ${totalDimensions}`);
                } else {
                    console.log(`[${new Date().toISOString()}] Unexpected result format:`, result);
                }
            }
            
            const elapsed = (Date.now() - startTime) / 1000;
            const chunksPerSecond = processedChunks / elapsed;
            console.log(`[${new Date().toISOString()}] Processed ${processedChunks}/${testChunks.length} chunks (${chunksPerSecond.toFixed(2)} chunks/sec, ${chunkTime}ms)`);
        }
        
        // Cleanup
        await embedder.cleanup();
        
        const totalTime = (Date.now() - startTime) / 1000;
        const chunksPerSecond = processedChunks / totalTime;
        
        console.log(`\n[${new Date().toISOString()}] === BGE RESULTS ===`);
        console.log(`Model: BGE-small-en-v1.5`);
        console.log(`Processed chunks: ${processedChunks}/${testChunks.length}`);
        console.log(`Total time: ${totalTime.toFixed(2)}s`);
        console.log(`Chunks per second: ${chunksPerSecond.toFixed(2)}`);
        console.log(`Average time per chunk: ${(totalTime / processedChunks * 1000).toFixed(2)}ms`);
        console.log(`First embedding time: ${firstEmbeddingTime}ms`);
        console.log(`Embedding dimensions: ${totalDimensions}`);
        console.log(`Initialization time: ${initTime}ms`);
        
        return {
            model: 'BGE-small-en-v1.5',
            processedChunks,
            totalTime,
            chunksPerSecond,
            avgTimePerChunk: totalTime / processedChunks * 1000,
            dimensions: totalDimensions,
            firstEmbeddingTime,
            initTime
        };
        
    } catch (error) {
        console.error(`[${new Date().toISOString()}] BGE test failed:`, error.message);
        console.error(`[${new Date().toISOString()}] Stack trace:`, error.stack);
        return null;
    }
}

// Run the test
runBGETest().catch(console.error);
