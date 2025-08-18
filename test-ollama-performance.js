#!/usr/bin/env node

const { spawn } = require('child_process');
const fs = require('fs');
const path = require('path');

// Sample code chunks for testing (similar to what your system processes)
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
    "function quickSort(arr) { if (arr.length <= 1) return arr; const pivot = arr[Math.floor(arr.length / 2)]; const left = arr.filter(x => x < pivot); const right = arr.filter(x => x > pivot); return [...quickSort(left), pivot, ...quickSort(right)]; }"
];

// Extend test chunks to get more meaningful performance data
const extendedChunks = [];
for (let i = 0; i < 50; i++) {
    extendedChunks.push(...testChunks);
}

console.log(`[${new Date().toISOString()}] Starting embedding performance comparison`);
console.log(`[${new Date().toISOString()}] Test chunks: ${extendedChunks.length}`);

// Test Ollama nomic-embed-text
async function testOllamaEmbedding() {
    console.log(`\n[${new Date().toISOString()}] === Testing Ollama nomic-embed-text ===`);
    
    const startTime = Date.now();
    let processedChunks = 0;
    let totalDimensions = 0;
    
    for (const chunk of extendedChunks) {
        try {
            const chunkStartTime = Date.now();
            
            // Call Ollama embedding API
            const result = await new Promise((resolve, reject) => {
                const ollama = spawn('ollama', ['run', 'nomic-embed-text:latest'], {
                    stdio: ['pipe', 'pipe', 'pipe']
                });
                
                let output = '';
                let errorOutput = '';
                
                ollama.stdout.on('data', (data) => {
                    output += data.toString();
                });
                
                ollama.stderr.on('data', (data) => {
                    errorOutput += data.toString();
                });
                
                ollama.on('close', (code) => {
                    if (code === 0) {
                        resolve(output);
                    } else {
                        reject(new Error(`Ollama process exited with code ${code}: ${errorOutput}`));
                    }
                });
                
                // Send the text to embed
                ollama.stdin.write(chunk);
                ollama.stdin.end();
            });
            
            const chunkTime = Date.now() - chunkStartTime;
            processedChunks++;
            
            // Try to extract embedding dimensions (Ollama output format varies)
            if (processedChunks === 1) {
                console.log(`[${new Date().toISOString()}] First chunk processed in ${chunkTime}ms`);
                console.log(`[${new Date().toISOString()}] Sample output length: ${output.length} chars`);
            }
            
            if (processedChunks % 10 === 0) {
                const elapsed = (Date.now() - startTime) / 1000;
                const chunksPerSecond = processedChunks / elapsed;
                console.log(`[${new Date().toISOString()}] Processed ${processedChunks}/${extendedChunks.length} chunks (${chunksPerSecond.toFixed(2)} chunks/sec)`);
            }
            
        } catch (error) {
            console.error(`[${new Date().toISOString()}] Error processing chunk ${processedChunks + 1}:`, error.message);
            break;
        }
    }
    
    const totalTime = (Date.now() - startTime) / 1000;
    const chunksPerSecond = processedChunks / totalTime;
    
    return {
        model: 'nomic-embed-text:latest',
        processedChunks,
        totalTime,
        chunksPerSecond,
        avgTimePerChunk: totalTime / processedChunks * 1000
    };
}

// Test BGE via your existing system (if available)
async function testBGEEmbedding() {
    console.log(`\n[${new Date().toISOString()}] === Testing BGE-small-en-v1.5 (Your Current) ===`);
    
    try {
        // Check if your embedder is available
        const { EmbeddingGenerator } = require('./dist/embedder.js');
        
        const startTime = Date.now();
        const embedder = new EmbeddingGenerator();
        
        // Initialize embedder
        await embedder.initialize();
        
        let processedChunks = 0;
        
        for (const chunk of extendedChunks) {
            const chunkStartTime = Date.now();
            
            const embedding = await embedder.generateEmbedding(chunk);
            const chunkTime = Date.now() - chunkStartTime;
            
            processedChunks++;
            
            if (processedChunks === 1) {
                console.log(`[${new Date().toISOString()}] First chunk processed in ${chunkTime}ms`);
                console.log(`[${new Date().toISOString()}] Embedding dimensions: ${embedding.length}`);
            }
            
            if (processedChunks % 10 === 0) {
                const elapsed = (Date.now() - startTime) / 1000;
                const chunksPerSecond = processedChunks / elapsed;
                console.log(`[${new Date().toISOString()}] Processed ${processedChunks}/${extendedChunks.length} chunks (${chunksPerSecond.toFixed(2)} chunks/sec)`);
            }
        }
        
        const totalTime = (Date.now() - startTime) / 1000;
        const chunksPerSecond = processedChunks / totalTime;
        
        return {
            model: 'BGE-small-en-v1.5',
            processedChunks,
            totalTime,
            chunksPerSecond,
            avgTimePerChunk: totalTime / processedChunks * 1000
        };
        
    } catch (error) {
        console.log(`[${new Date().toISOString()}] BGE test skipped: ${error.message}`);
        return null;
    }
}

// Run the comparison
async function runComparison() {
    try {
        // Test Ollama first (limit to smaller batch for initial test)
        const smallTestChunks = testChunks.slice(0, 10); // Just 10 chunks for quick test
        console.log(`[${new Date().toISOString()}] Running quick test with ${smallTestChunks.length} chunks`);
        
        // Override extendedChunks for quick test
        const originalChunks = [...extendedChunks];
        extendedChunks.length = 0;
        extendedChunks.push(...smallTestChunks);
        
        const ollamaResults = await testOllamaEmbedding();
        
        // Restore original chunks for BGE test if available
        extendedChunks.length = 0;
        extendedChunks.push(...smallTestChunks);
        
        const bgeResults = await testBGEEmbedding();
        
        // Print comparison results
        console.log(`\n[${new Date().toISOString()}] === PERFORMANCE COMPARISON ===`);
        console.log(`Ollama nomic-embed-text:`);
        console.log(`  - Chunks/second: ${ollamaResults.chunksPerSecond.toFixed(2)}`);
        console.log(`  - Avg time per chunk: ${ollamaResults.avgTimePerChunk.toFixed(2)}ms`);
        console.log(`  - Total time: ${ollamaResults.totalTime.toFixed(2)}s`);
        
        if (bgeResults) {
            console.log(`\nBGE-small-en-v1.5 (Current):`);
            console.log(`  - Chunks/second: ${bgeResults.chunksPerSecond.toFixed(2)}`);
            console.log(`  - Avg time per chunk: ${bgeResults.avgTimePerChunk.toFixed(2)}ms`);
            console.log(`  - Total time: ${bgeResults.totalTime.toFixed(2)}s`);
            
            const speedRatio = bgeResults.chunksPerSecond / ollamaResults.chunksPerSecond;
            console.log(`\nSpeed comparison: BGE is ${speedRatio.toFixed(2)}x ${speedRatio > 1 ? 'faster' : 'slower'} than Ollama`);
        }
        
    } catch (error) {
        console.error(`[${new Date().toISOString()}] Test failed:`, error);
    }
}

// Run the test
runComparison();
