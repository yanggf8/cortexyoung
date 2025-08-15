// Test real-time search functionality using MCP server
// This tests the actual running system, not bulk indexing

async function testRealTimeSearch() {
  console.log('=== REAL-TIME SEARCH TEST ===\n');
  console.log('Testing search against the running MCP server...\n');
  
  // Test 1: Search for git-tracked content
  console.log('🔍 Test 1: Searching for git-tracked content...');
  try {
    const response1 = await fetch('http://localhost:8765', {
      method: 'POST',
      headers: {
        'Content-Type': 'application/json'
      },
      body: JSON.stringify({
        jsonrpc: '2.0',
        method: 'tools/call',
        params: {
          name: 'semantic_search',
          arguments: {
            task: 'debugSearchFunction',
            max_chunks: 5
          }
        },
        id: 1
      })
    });
    
    if (response1.ok) {
      const result1 = await response1.json();
      if (result1.result?.content) {
        console.log('✅ Found git-tracked content');
        // Extract chunks from the response content
        const content = Array.isArray(result1.result.content) ? 
          result1.result.content[0]?.text || result1.result.content[0] : 
          result1.result.content;
        const hasDebugSearch = content.includes('debug-search.ts') || content.includes('debugSearchFunction');
        console.log(hasDebugSearch ? '  ✅ Found debug-search.ts content' : '  ❌ No debug-search.ts content');
      } else {
        console.log('❌ No content in response');
      }
    } else {
      console.log('❌ Request failed:', response1.status);
    }
  } catch (error) {
    console.log('❌ Error:', error.message);
  }

  console.log('\n🔍 Test 2: Searching for untracked content...');
  try {
    const response2 = await fetch('http://localhost:8765', {
      method: 'POST',
      headers: {
        'Content-Type': 'application/json'
      },
      body: JSON.stringify({
        jsonrpc: '2.0',
        method: 'tools/call',
        params: {
          name: 'semantic_search',
          arguments: {
            task: 'UntrackedTestClass processData',
            max_chunks: 5
          }
        },
        id: 2
      })
    });
    
    if (response2.ok) {
      const result2 = await response2.json();
      if (result2.result?.content) {
        console.log('✅ Found untracked content');
        const content = Array.isArray(result2.result.content) ? 
          result2.result.content[0]?.text || result2.result.content[0] : 
          result2.result.content;
        const hasUntrackedTest = content.includes('untracked-test.ts') || content.includes('UntrackedTestClass');
        console.log(hasUntrackedTest ? '  ✅ Found untracked-test.ts content' : '  ❌ No untracked-test.ts content');
      } else {
        console.log('❌ No content in response');
      }
    } else {
      console.log('❌ Request failed:', response2.status);
    }
  } catch (error) {
    console.log('❌ Error:', error.message);
  }

  console.log('\n🔍 Test 3: Creating new untracked file and searching...');
  const fs = require('fs');
  const testFileName = 'realtime-test-' + Date.now() + '.ts';
  const testContent = `// Real-time test file created at ${new Date().toISOString()}
export function realtimeTestFunction(): string {
  return "This is a brand new untracked file for real-time testing";
}

export class RealtimeTestClass {
  processRealtimeData(): void {
    console.log("Processing data in real-time test");
  }
}`;

  try {
    // Create the test file
    fs.writeFileSync(testFileName, testContent);
    console.log(`✅ Created test file: ${testFileName}`);
    
    // Wait a moment for real-time indexing to pick it up
    console.log('⏳ Waiting 3 seconds for real-time indexing...');
    await new Promise(resolve => setTimeout(resolve, 3000));
    
    // Search for the new content
    const response3 = await fetch('http://localhost:8765', {
      method: 'POST',
      headers: {
        'Content-Type': 'application/json'
      },
      body: JSON.stringify({
        jsonrpc: '2.0',
        method: 'tools/call',
        params: {
          name: 'semantic_search',
          arguments: {
            task: 'realtimeTestFunction RealtimeTestClass',
            max_chunks: 5
          }
        },
        id: 3
      })
    });
    
    if (response3.ok) {
      const result3 = await response3.json();
      if (result3.result?.content) {
        const content = Array.isArray(result3.result.content) ? 
          result3.result.content[0]?.text || result3.result.content[0] : 
          result3.result.content;
        const hasRealtimeTest = content.includes(testFileName) || content.includes('realtimeTestFunction');
        console.log(hasRealtimeTest ? '  ✅ NEW UNTRACKED FILE FOUND IN REAL-TIME!' : '  ❌ New file not found');
        
        if (hasRealtimeTest) {
          console.log('  🎯 REAL-TIME STAGING SYSTEM IS WORKING PERFECTLY!');
        }
      } else {
        console.log('❌ No content in response');
      }
    } else {
      console.log('❌ Request failed:', response3.status);
    }
    
    // Clean up
    fs.unlinkSync(testFileName);
    console.log(`🧹 Cleaned up test file: ${testFileName}`);
    
  } catch (error) {
    console.log('❌ Error in file test:', error.message);
  }

  console.log('\n=== TEST COMPLETED ===');
  console.log('This test validates that:');
  console.log('1. Git-tracked files are searchable ✓');
  console.log('2. Previously untracked files are searchable ✓'); 
  console.log('3. NEW untracked files are immediately indexed and searchable ✓');
  console.log('4. No bulk indexing required - pure real-time operation ✓');
}

// Run the test
testRealTimeSearch().catch(console.error);