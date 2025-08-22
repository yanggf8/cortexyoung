#!/usr/bin/env ts-node

/**
 * V3.0 Implementation Test Script
 * 
 * Tests the core V3.0 components:
 * - Project Context Detection
 * - Context Enhancement Layer
 * - MCP Client HTTP communication
 * - Centralized Embedding Server
 */

import { ProjectContextDetector } from './src/project-context-detector';
import { ContextEnhancer } from './src/context-enhancer';
import { CortexMCPClient } from './src/cortex-mcp-client';
import { conditionalLogger } from './src/utils/console-logger';

async function testProjectContextDetection() {
  console.log('\n🔍 Testing Project Context Detection...');
  
  const detector = new ProjectContextDetector();
  const currentProject = process.cwd();
  
  try {
    const context = await detector.detectProjectContext(currentProject);
    
    console.log('✅ Project Context Detected:');
    console.log(`   Type: ${context.type}`);
    console.log(`   Language: ${context.language}`);
    console.log(`   Framework: ${context.framework}`);
    console.log(`   Directories: ${context.directories.join(', ')}`);
    console.log(`   Dependencies: ${context.dependencies.join(', ')}`);
    console.log(`   Confidence: ${(context.confidence * 100).toFixed(1)}%`);
    
    if (Object.keys(context.patterns).length > 0) {
      console.log('   Patterns:');
      Object.entries(context.patterns).forEach(([key, value]) => {
        console.log(`     ${key}: ${value}`);
      });
    }
    
    return true;
  } catch (error) {
    console.error('❌ Project context detection failed:', error);
    return false;
  }
}

async function testContextEnhancement() {
  console.log('\n🎨 Testing Context Enhancement...');
  
  const enhancer = new ContextEnhancer();
  const currentProject = process.cwd();
  
  try {
    // Test enhancement with mock semantic results
    const mockResults = `function authenticateUser(req, res, next) {
  const token = req.headers.authorization;
  if (!token) {
    return res.status(401).json({ error: 'No token provided' });
  }
  
  try {
    const decoded = jwt.verify(token, JWT_SECRET);
    req.user = decoded;
    next();
  } catch (error) {
    return res.status(403).json({ error: 'Invalid token' });
  }
}`;

    const enhancement = await enhancer.enhanceSemanticResults(
      mockResults,
      'JWT authentication middleware',
      currentProject
    );
    
    console.log('✅ Context Enhancement Results:');
    console.log(`   Enhanced: ${enhancement.stats.enhanced}`);
    console.log(`   Context Type: ${enhancement.stats.contextType}`);
    console.log(`   Tokens Added: ${enhancement.stats.tokensAdded}`);
    console.log(`   Confidence: ${(enhancement.stats.confidence * 100).toFixed(1)}%`);
    
    console.log('\n📋 Enhanced Results Preview:');
    console.log(enhancement.results.substring(0, 300) + '...');
    
    return true;
  } catch (error) {
    console.error('❌ Context enhancement failed:', error);
    return false;
  }
}

async function testMCPClient() {
  console.log('\n🔌 Testing MCP Client...');
  
  const client = new CortexMCPClient(process.cwd(), {
    serverUrl: 'http://localhost:3001',
    fallbackEnabled: true
  });
  
  try {
    // Test server status
    console.log('📡 Testing server connection...');
    const status = await client.getServerStatus();
    console.log(`   Server Status: ${status.status || 'unavailable'}`);
    
    // Test context enhancement
    console.log('🧪 Testing context enhancement...');
    const testResult = await client.testContextEnhancement();
    
    if (testResult.error) {
      console.log('⚠️  Server unavailable - testing fallback mode');
      
      // Test fallback semantic search
      const fallbackResult = await client.semanticSearch('authentication logic');
      console.log('✅ Fallback search working');
      console.log(`   Result preview: ${fallbackResult.substring(0, 100)}...`);
    } else {
      console.log('✅ Enhanced search working');
      console.log(`   Enhanced: ${testResult.enhanced}`);
      console.log(`   Preview: ${testResult.preview.substring(0, 100)}...`);
    }
    
    // Get client stats
    const clientStats = client.getClientStats();
    console.log('\n📊 Client Statistics:');
    console.log(`   Fallback Mode: ${clientStats.fallbackMode}`);
    console.log(`   Connection Failures: ${clientStats.connectionFailures}`);
    console.log(`   Server Reachable: ${clientStats.serverReachable}`);
    
    return true;
  } catch (error) {
    console.error('❌ MCP client test failed:', error);
    return false;
  }
}

async function runV3Tests() {
  console.log('🚀 Cortex V3.0 Implementation Test Suite');
  console.log('━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━');
  
  const results = {
    contextDetection: await testProjectContextDetection(),
    contextEnhancement: await testContextEnhancement(),
    mcpClient: await testMCPClient()
  };
  
  console.log('\n📋 Test Results Summary:');
  console.log('━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━');
  
  Object.entries(results).forEach(([test, passed]) => {
    const status = passed ? '✅ PASS' : '❌ FAIL';
    const testName = test.replace(/([A-Z])/g, ' $1').replace(/^./, str => str.toUpperCase());
    console.log(`${status} ${testName}`);
  });
  
  const allPassed = Object.values(results).every(result => result);
  
  if (allPassed) {
    console.log('\n🎉 All V3.0 Core Components Working!');
    console.log('Ready for integration testing with embedding server.');
  } else {
    console.log('\n⚠️  Some components need attention before proceeding.');
  }
  
  return allPassed;
}

// Run tests if script is executed directly
if (require.main === module) {
  runV3Tests().then(success => {
    process.exit(success ? 0 : 1);
  }).catch(error => {
    console.error('Test suite crashed:', error);
    process.exit(1);
  });
}

export { runV3Tests };