#!/usr/bin/env node

const { CallGraphAnalyzer } = require('./dist/call-graph-analyzer');
const Parser = require('tree-sitter');
const JavaScript = require('tree-sitter-javascript');
const TypeScript = require('tree-sitter-typescript');

async function testAnalyzerDetailed() {
    console.log('🔬 Detailed analyzer debugging...');
    
    // Test simple code sample first
    const simpleCode = `
function testFunction() {
    return "hello";
}

class TestClass {
    constructor() {
        this.value = 42;
    }
    
    method() {
        testFunction();
    }
}

export { TestClass };
import { something } from './other';
`;

    console.log('\n=== Testing with simple code ===');
    console.log(simpleCode);
    
    // Test tree-sitter parsing directly
    console.log('\n=== Testing tree-sitter parsing ===');
    try {
        const parser = new Parser();
        parser.setLanguage(TypeScript.typescript);
        
        const tree = parser.parse(simpleCode);
        console.log(`Parse successful: ${!tree.rootNode.hasError}`);
        console.log(`Root node type: ${tree.rootNode.type}`);
        console.log(`Child count: ${tree.rootNode.childCount}`);
        
        // Walk through nodes
        const walkNode = (node, depth = 0) => {
            if (depth > 3) return; // Limit depth
            const indent = '  '.repeat(depth);
            console.log(`${indent}${node.type}: "${node.text?.substring(0, 50)?.replace(/\n/g, '\\n')}"`);
            
            for (let i = 0; i < Math.min(node.childCount, 5); i++) {
                walkNode(node.child(i), depth + 1);
            }
        };
        
        walkNode(tree.rootNode);
        
    } catch (error) {
        console.error('Tree-sitter parsing failed:', error.message);
    }
    
    // Test CallGraphAnalyzer
    console.log('\n=== Testing CallGraphAnalyzer ===');
    try {
        const analyzer = new CallGraphAnalyzer();
        const result = await analyzer.analyzeFile('test.ts', simpleCode);
        
        console.log(`Analysis result:`);
        console.log(`  - Symbols: ${result.symbols.length}`);
        console.log(`  - Relationships: ${result.relationships.length}`);
        console.log(`  - Imports: ${result.imports.length}`);
        console.log(`  - Exports: ${result.exports.length}`);
        console.log(`  - Errors: ${result.errors.length}`);
        
        if (result.errors.length > 0) {
            console.log('\nErrors:');
            result.errors.forEach(err => {
                console.log(`  - ${err.type}: ${err.message}`);
            });
        }
        
        if (result.symbols.length > 0) {
            console.log('\nSymbols:');
            result.symbols.forEach(symbol => {
                console.log(`  - ${symbol.name} (${symbol.type}) at line ${symbol.startLine}`);
            });
        }
        
        if (result.relationships.length > 0) {
            console.log('\nRelationships:');
            result.relationships.forEach(rel => {
                console.log(`  - ${rel.type}: ${rel.fromSymbol} -> ${rel.toSymbol}`);
            });
        }
        
        if (result.imports.length > 0) {
            console.log('\nImports:');
            result.imports.forEach(imp => {
                console.log(`  - ${imp.importType} from "${imp.source}": ${imp.importedSymbols.join(', ')}`);
            });
        }
        
        if (result.exports.length > 0) {
            console.log('\nExports:');
            result.exports.forEach(exp => {
                console.log(`  - ${exp.exportType}: ${exp.exportedSymbol}`);
            });
        }
        
    } catch (error) {
        console.error('CallGraphAnalyzer failed:', error.message);
        console.error('Stack:', error.stack);
    }
}

testAnalyzerDetailed().catch(console.error);