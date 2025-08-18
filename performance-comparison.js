#!/usr/bin/env node

const http = require('http');

// Test chunks
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

console.log(`[${new Date().toISOString()}] Starting embedding performance comparison`);
console.log(`[${new Date().toISOString()}] Test chunks: ${testChunks.length}`);

// Ollama embedding function
async function callOllamaEmbedding(text) {
    return new Promise((resolve, reject) => {
        const postData = JSON.stringify({
            model: 'nomic-embed-text:latest',
            prompt: text
        });

        const options = {
            hostname: 'localhost',
            port: 11434,
            path: '/api/embeddings',
            method: 'POST',
            headers: {
                'Content-Type': 'application/json',
                'Content-Length': Buffer.byteLength(postData)
            }
        };

        const req = http.request(options, (res) => {
            let data = '';
            
            res.on('data', (chunk) => {
                data += chunk;
            });
            
            res.on('end', () => {
                try {
                    const response = JSON.parse(data);
                    resolve(response);
                } catch (error) {
                    reject(new Error(`Failed to parse response: ${error.message}`));
                }
            });
        });

        req.on('error', (error) => {
            reject(error);
        });

        req.write(postData);
        req.end();
    });
}

// Test Ollama
async function testOllama() {
    console.log(`\n[${new Date().toISOString()}] === Testing Ollama nomic-embed-text ===`);
    
    const startTime = Date.now();
    let processedChunks = 0;
    let firstEmbeddingTime = 0;
    let totalDimensions = 0;

    for (const chunk of testChunks) {
        try {
            const chunkStartTime = Date.now();
            const result = await callOllamaEmbedding(chunk);
            const chunkTime = Date.now() - chunkStartTime;
            
            processedChunks++;
            
            if (processedChunks === 1) {
                firstEmbeddingTime = chunkTime;
                if (result.embedding) {
                    totalDimensions = result.embedding.length;
                    console.log(`[${new Date().toISOString()}] First chunk: ${chunkTime}ms, dimensions: ${totalDimensions}`);
                }
            }
            
            if (processedChunks % 3 === 0 || processedChunks === testChunks.length) {
                const elapsed = (Date.now() - startTime) / 1000;
                const chunksPerSecond = processedChunks / elapsed;
                console.log(`[${new Date().toISOString()}] Ollama: ${processedChunks}/${testChunks.length} (${chunksPerSecond.toFixed(2)} chunks/sec)`);
            }
            
        } catch (error) {
            console.error(`[${new Date().toISOString()}] Ollama error:`, error.message);
            break;
        }
    }
    
    const totalTime = (Date.now() - startTime) / 1000;
    return {
        model: 'nomic-embed-text:latest',
        processedChunks,
        totalTime,
        chunksPerSecond: processedChunks / totalTime,
        avgTimePerChunk: totalTime / processedChunks * 1000,
        dimensions: totalDimensions,
        firstEmbeddingTime
    };
}

// Test BGE
async function testBGE() {
    console.log(`\n[${new Date().toISOString()}] === Testing BGE-small-en-v1.5 ===`);
    
    try {
        const { ProcessPoolEmbedder } = require('./dist/process-pool-embedder.js');
        
        const startTime = Date.now();
        const embedder = new ProcessPoolEmbedder();
        
        console.log(`[${new Date().toISOString()}] Initializing BGE...`);
        const initStart = Date.now();
        await embedder.initialize();
        const initTime = Date.now() - initStart;
        console.log(`[${new Date().toISOString()}] BGE initialized in ${initTime}ms`);
        
        let processedChunks = 0;
        let firstEmbeddingTime = 0;
        let totalDimensions = 0;
        
        for (const chunk of testChunks) {
            const chunkStartTime = Date.now();
            const result = await embedder.embedBatch([chunk]);
            const chunkTime = Date.now() - chunkStartTime;
            
            processedChunks++;
            
            if (processedChunks === 1) {
                firstEmbeddingTime = chunkTime;
                if (result.embeddings && result.embeddings.length > 0) {
                    totalDimensions = result.embeddings[0].length;
                    console.log(`[${new Date().toISOString()}] First chunk: ${chunkTime}ms, dimensions: ${totalDimensions}`);
                }
            }
            
            if (processedChunks % 3 === 0 || processedChunks === testChunks.length) {
                const elapsed = (Date.now() - startTime) / 1000;
                const chunksPerSecond = processedChunks / elapsed;
                console.log(`[${new Date().toISOString()}] BGE: ${processedChunks}/${testChunks.length} (${chunksPerSecond.toFixed(2)} chunks/sec)`);
            }
        }
        
        const totalTime = (Date.now() - startTime) / 1000;
        return {
            model: 'BGE-small-en-v1.5',
            processedChunks,
            totalTime,
            chunksPerSecond: processedChunks / totalTime,
            avgTimePerChunk: totalTime / processedChunks * 1000,
            dimensions: totalDimensions,
            firstEmbeddingTime,
            initTime
        };
        
    } catch (error) {
        console.error(`[${new Date().toISOString()}] BGE test failed:`, error.message);
        return null;
    }
}

// Run comparison
async function runComparison() {
    try {
        const ollamaResults = await testOllama();
        const bgeResults = await testBGE();
        
        console.log(`\n[${new Date().toISOString()}] ═══ PERFORMANCE COMPARISON ═══`);
        
        console.log(`\nOllama nomic-embed-text:`);
        console.log(`  📊 Chunks/second: ${ollamaResults.chunksPerSecond.toFixed(2)}`);
        console.log(`  ⏱️  Avg per chunk: ${ollamaResults.avgTimePerChunk.toFixed(0)}ms`);
        console.log(`  🎯 Dimensions: ${ollamaResults.dimensions}`);
        console.log(`  🚀 First chunk: ${ollamaResults.firstEmbeddingTime}ms`);
        
        if (bgeResults) {
            console.log(`\nBGE-small-en-v1.5 (Your Current):`);
            console.log(`  📊 Chunks/second: ${bgeResults.chunksPerSecond.toFixed(2)}`);
            console.log(`  ⏱️  Avg per chunk: ${bgeResults.avgTimePerChunk.toFixed(0)}ms`);
            console.log(`  🎯 Dimensions: ${bgeResults.dimensions}`);
            console.log(`  🚀 First chunk: ${bgeResults.firstEmbeddingTime}ms`);
            console.log(`  🔧 Init time: ${bgeResults.initTime}ms`);
            
            const speedRatio = bgeResults.chunksPerSecond / ollamaResults.chunksPerSecond;
            const memoryRatio = bgeResults.dimensions / ollamaResults.dimensions;
            
            console.log(`\n🏆 WINNER ANALYSIS:`);
            console.log(`  Speed: BGE is ${speedRatio.toFixed(2)}x ${speedRatio > 1 ? 'FASTER' : 'slower'} than Ollama`);
            console.log(`  Memory: BGE uses ${(memoryRatio * 100).toFixed(0)}% of Ollama's vector memory`);
            console.log(`  Startup: BGE has ${bgeResults.initTime}ms initialization overhead`);
            
            if (speedRatio > 1.5) {
                console.log(`  🎯 RECOMMENDATION: Stick with BGE - significantly faster`);
            } else if (speedRatio < 0.7) {
                console.log(`  🎯 RECOMMENDATION: Consider Ollama - notably faster`);
            } else {
                console.log(`  🎯 RECOMMENDATION: Performance similar - consider other factors`);
            }
        }
        
    } catch (error) {
        console.error(`[${new Date().toISOString()}] Comparison failed:`, error);
    }
}

runComparison();
