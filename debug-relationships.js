#!/usr/bin/env node

const { RelationshipTraversalEngine } = require('./dist/relationship-traversal-engine');
const { CallGraphAnalyzer } = require('./dist/call-graph-analyzer');  
const path = require('path');
const fs = require('fs');

async function testRelationshipBuilding() {
    console.log('🔍 Testing relationship building on sample files...');
    
    // Create test engine
    const repositoryPath = '/home/yanggf/a/cortexyoung';
    const engine = new RelationshipTraversalEngine(repositoryPath);
    
    // Get a small sample of files
    const testFiles = new Map();
    
    // Add searcher.ts - should have lots of imports and calls
    const searcherPath = path.join(repositoryPath, 'src/searcher.ts');
    const searcherContent = fs.readFileSync(searcherPath, 'utf-8');
    testFiles.set(searcherPath, searcherContent);
    
    // Add indexer.ts - should have class and method relationships  
    const indexerPath = path.join(repositoryPath, 'src/indexer.ts');
    const indexerContent = fs.readFileSync(indexerPath, 'utf-8');
    testFiles.set(indexerPath, indexerContent);
    
    console.log(`Testing with ${testFiles.size} files...`);
    
    // Test individual analyzer first
    console.log('\n=== Testing CallGraphAnalyzer directly ===');
    const analyzer = new CallGraphAnalyzer();
    
    try {
        const searcherAnalysis = await analyzer.analyzeFile(searcherPath, searcherContent);
        console.log(`Searcher analysis:`);
        console.log(`  - Symbols: ${searcherAnalysis.symbols.length}`);
        console.log(`  - Relationships: ${searcherAnalysis.relationships.length}`);
        console.log(`  - Imports: ${searcherAnalysis.imports.length}`);
        console.log(`  - Exports: ${searcherAnalysis.exports.length}`);
        
        if (searcherAnalysis.symbols.length > 0) {
            console.log(`  - Sample symbol: ${searcherAnalysis.symbols[0].name} (${searcherAnalysis.symbols[0].type})`);
        }
        
        if (searcherAnalysis.relationships.length > 0) {
            console.log(`  - Sample relationship: ${searcherAnalysis.relationships[0].type} (${searcherAnalysis.relationships[0].fromSymbol} -> ${searcherAnalysis.relationships[0].toSymbol})`);
        }
        
    } catch (error) {
        console.error('CallGraphAnalyzer failed:', error.message);
    }
    
    console.log('\n=== Testing RelationshipTraversalEngine ===');
    
    try {
        // Test full relationship building
        await engine.buildRelationshipGraph(testFiles);
        
        // Get statistics
        const stats = engine.getGraphStatistics();
        console.log('Relationship Graph Statistics:');
        console.log(`  - Total Symbols: ${stats.totalSymbols}`);
        console.log(`  - Total Relationships: ${stats.totalRelationships}`);
        console.log(`  - Average Connections: ${stats.averageSymbolConnections}`);
        
        // Show symbol type distribution
        console.log('\nSymbol Type Distribution:');
        for (const [type, count] of stats.symbolTypeDistribution) {
            console.log(`  - ${type}: ${count}`);
        }
        
        // Show relationship type distribution
        console.log('\nRelationship Type Distribution:');  
        for (const [type, count] of stats.relationshipTypeDistribution) {
            console.log(`  - ${type}: ${count}`);
        }
        
        // Test symbol queries
        const graph = engine.getGraph();
        if (graph.symbols.size > 0) {
            console.log('\nSample Symbols:');
            let count = 0;
            for (const symbol of graph.symbols.values()) {
                if (count >= 5) break;
                console.log(`  - ${symbol.name} (${symbol.type}) in ${symbol.filePath}:${symbol.startLine}`);
                count++;
            }
        }
        
        if (graph.relationships.size > 0) {
            console.log('\nSample Relationships:');
            let count = 0;
            for (const rel of graph.relationships.values()) {
                if (count >= 5) break;
                console.log(`  - ${rel.type}: ${rel.fromSymbol} -> ${rel.toSymbol} (strength: ${rel.strength})`);
                count++;
            }
        }
        
    } catch (error) {
        console.error('RelationshipTraversalEngine failed:', error.message);
        console.error('Stack:', error.stack);
    }
}

// Run the test
testRelationshipBuilding().catch(console.error);