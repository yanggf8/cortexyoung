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

console.log(`[${new Date().toISOString()}] Starting Ollama embedding performance test`);
console.log(`[${new Date().toISOString()}] Test chunks: ${testChunks.length}`);

// Function to call Ollama embeddings API
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

// Run the test
async function runOllamaTest() {
    console.log(`\n[${new Date().toISOString()}] === Testing Ollama nomic-embed-text ===`);
    
    const startTime = Date.now();
    let processedChunks = 0;
    let totalDimensions = 0;
    let firstEmbeddingTime = 0;

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
                    console.log(`[${new Date().toISOString()}] First chunk processed in ${chunkTime}ms`);
                    console.log(`[${new Date().toISOString()}] Embedding dimensions: ${totalDimensions}`);
                } else {
                    console.log(`[${new Date().toISOString()}] Response:`, JSON.stringify(result, null, 2));
                }
            }
            
            const elapsed = (Date.now() - startTime) / 1000;
            const chunksPerSecond = processedChunks / elapsed;
            console.log(`[${new Date().toISOString()}] Processed ${processedChunks}/${testChunks.length} chunks (${chunksPerSecond.toFixed(2)} chunks/sec, ${chunkTime}ms)`);
            
        } catch (error) {
            console.error(`[${new Date().toISOString()}] Error processing chunk ${processedChunks + 1}:`, error.message);
            break;
        }
    }
    
    const totalTime = (Date.now() - startTime) / 1000;
    const chunksPerSecond = processedChunks / totalTime;
    
    console.log(`\n[${new Date().toISOString()}] === OLLAMA RESULTS ===`);
    console.log(`Model: nomic-embed-text:latest`);
    console.log(`Processed chunks: ${processedChunks}/${testChunks.length}`);
    console.log(`Total time: ${totalTime.toFixed(2)}s`);
    console.log(`Chunks per second: ${chunksPerSecond.toFixed(2)}`);
    console.log(`Average time per chunk: ${(totalTime / processedChunks * 1000).toFixed(2)}ms`);
    console.log(`First embedding time: ${firstEmbeddingTime}ms`);
    console.log(`Embedding dimensions: ${totalDimensions}`);
    
    return {
        model: 'nomic-embed-text:latest',
        processedChunks,
        totalTime,
        chunksPerSecond,
        avgTimePerChunk: totalTime / processedChunks * 1000,
        dimensions: totalDimensions,
        firstEmbeddingTime
    };
}

// Run the test
runOllamaTest().catch(console.error);
