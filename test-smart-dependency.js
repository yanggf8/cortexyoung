#!/usr/bin/env node
/**
 * Test script to demonstrate Smart Dependency Chain functionality
 * Shows how the system automatically includes complete dependency context
 */

const http = require('http');

async function testSmartDependencyChain() {
  console.log('🔬 Testing Smart Dependency Chain for Context Window Optimization');
  console.log('===============================================================\n');

  const testCases = [
    {
      name: 'Function X with Complete Dependencies',
      query: 'semantic search functionality for code intelligence',
      multi_hop: {
        enabled: true,
        max_hops: 2,
        relationship_types: ['calls', 'imports', 'data_flow']
      },
      token_budget: 6000
    },
    {
      name: 'Vector Store Operations with Dependencies', 
      query: 'vector store similarity search and embeddings',
      multi_hop: {
        enabled: true,
        max_hops: 3,
        relationship_types: ['calls', 'imports', 'extends']
      },
      token_budget: 8000
    },
    {
      name: 'Process Pool Management with Context',
      query: 'process pool embedder resource management',
      multi_hop: {
        enabled: true,
        max_hops: 2,
        relationship_types: ['calls', 'configures', 'depends_on']
      },
      token_budget: 5000
    }
  ];

  for (const testCase of testCases) {
    console.log(`\n🎯 Test Case: ${testCase.name}`);
    console.log(`   Query: "${testCase.query}"`);
    console.log(`   Token Budget: ${testCase.token_budget}`);
    console.log(`   Max Hops: ${testCase.multi_hop.max_hops}`);
    console.log(`   Relationship Types: ${testCase.multi_hop.relationship_types.join(', ')}`);
    console.log('   ─────────────────────────────────────────────────────────────');

    try {
      const response = await makeRequest('/mcp', {
        method: 'tools/call',
        params: {
          name: 'semantic_search',
          arguments: {
            query: testCase.query,
            max_chunks: 20,
            multi_hop: testCase.multi_hop,
            token_budget: testCase.token_budget
          }
        }
      });

      // Extract result from MCP response
      const result = response.result || {};
      
      if (result.status === 'success') {
        console.log(`   ✅ Status: ${result.status}`);
        console.log(`   📊 Total chunks found: ${result.total_chunks_considered}`);
        console.log(`   📦 Chunks returned: ${result.chunks?.length || 0}`);
        console.log(`   ⏱️  Query time: ${result.query_time_ms}ms`);
        console.log(`   🎯 Efficiency score: ${((result.efficiency_score || 0) * 100).toFixed(1)}%`);
        
        // Smart Dependency Chain specific metrics
        if (result.dependency_chain) {
          console.log(`   🔗 Dependency Analysis:`);
          console.log(`      Completeness Score: ${(result.dependency_chain.completeness_score * 100).toFixed(1)}%`);
          console.log(`      Total Dependencies: ${result.dependency_chain.total_dependencies}`);
          console.log(`      Relationship Paths: ${result.dependency_chain.relationship_paths?.length || 0}`);
        }

        if (result.metadata?.dependency_metrics) {
          const dm = result.metadata.dependency_metrics;
          console.log(`   📈 Dependency Breakdown:`);
          console.log(`      Critical: ${dm.critical_dependencies}`);
          console.log(`      Forward: ${dm.forward_dependencies}`);
          console.log(`      Backward: ${dm.backward_dependencies}`);
          console.log(`      Contextual: ${dm.contextual_dependencies}`);
        }

        if (result.context_package) {
          console.log(`   🧠 Context Package:`);
          console.log(`      Groups: ${result.context_package.groups?.length || 0}`);
          console.log(`      Total Tokens: ${result.context_package.total_tokens || 'N/A'}`);
          console.log(`      Token Efficiency: ${((result.context_package.token_efficiency || 0) * 100).toFixed(1)}%`);
          
          if (result.context_package.dependency_insights) {
            console.log(`   💡 Dependency Insights:`);
            result.context_package.dependency_insights.forEach((insight, i) => {
              console.log(`      ${i + 1}. ${insight}`);
            });
          }

          // Show context group breakdown
          if (result.context_package.groups) {
            console.log(`   📋 Context Groups:`);
            result.context_package.groups.forEach((group, i) => {
              const depType = group.dependency_type ? ` (${group.dependency_type})` : '';
              console.log(`      ${i + 1}. ${group.title}${depType}: ${group.chunks?.length || 0} chunks`);
            });
          }
        }

        if (result.metadata?.mmr_metrics) {
          const mmr = result.metadata.mmr_metrics;
          console.log(`   🎛️  MMR Optimization:`);
          console.log(`      Critical Coverage: ${(mmr.critical_set_coverage * 100).toFixed(1)}%`);
          console.log(`      Diversity Score: ${(mmr.diversity_score * 100).toFixed(1)}%`);
          console.log(`      Budget Utilization: ${(mmr.budget_utilization * 100).toFixed(1)}%`);
        }

      } else {
        console.log(`   ❌ Status: ${result.status || 'error'}`);
        console.log(`   Error: Request failed`);
      }
    } catch (error) {
      console.log(`   ❌ Error: ${error.message}`);
    }

    console.log('   ═════════════════════════════════════════════════════════════\n');
  }

  console.log('🎉 Smart Dependency Chain testing completed!');
  console.log('\n📝 Key Benefits Demonstrated:');
  console.log('   • Automatic inclusion of forward dependencies (what X calls)');
  console.log('   • Automatic inclusion of backward dependencies (what calls X)');
  console.log('   • Critical dependency prioritization (types, interfaces)');
  console.log('   • Token budget awareness and optimization');
  console.log('   • Context completeness scoring');
  console.log('   • MMR integration for diversity optimization');
}

function makeRequest(path, data) {
  return new Promise((resolve, reject) => {
    const postData = JSON.stringify(data);
    
    const options = {
      hostname: 'localhost',
      port: 8765,
      path: path,
      method: 'POST',
      headers: {
        'Content-Type': 'application/json',
        'Content-Length': Buffer.byteLength(postData)
      }
    };

    const req = http.request(options, (res) => {
      let responseBody = '';
      
      res.on('data', (chunk) => {
        responseBody += chunk;
      });
      
      res.on('end', () => {
        try {
          const response = JSON.parse(responseBody);
          resolve(response);
        } catch (e) {
          reject(new Error(`Failed to parse response: ${e.message}`));
        }
      });
    });

    req.on('error', (err) => {
      reject(err);
    });

    req.write(postData);
    req.end();
  });
}

// Run the test
if (require.main === module) {
  testSmartDependencyChain().catch(console.error);
}