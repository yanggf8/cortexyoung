#!/usr/bin/env node

/**
 * Quick test to validate relationship engine initialization is working
 */

const { CodebaseIndexer } = require('./dist/indexer');
const { StartupStageTracker } = require('./dist/startup-stages');

async function testRelationshipInitialization() {
  console.log('🧪 Testing Relationship Engine Initialization');
  console.log('============================================');
  
  const repoPath = process.cwd();
  const stageTracker = new StartupStageTracker();
  const indexer = new CodebaseIndexer(repoPath, stageTracker);
  
  try {
    console.log('📁 Repository:', repoPath);
    console.log('🔄 Starting incremental indexing to test relationship init...');
    
    const startTime = Date.now();
    
    // This should trigger relationship initialization
    const result = await indexer.indexRepository({
      repository_path: repoPath,
      mode: 'incremental'
    });
    
    const duration = Date.now() - startTime;
    
    console.log('\n✅ Indexing completed successfully!');
    console.log(`⏱️  Duration: ${duration}ms`);
    console.log(`📊 Chunks processed: ${result.chunks_processed}`);
    console.log(`✅ Status: ${result.status}`);
    
    // Check if relationship engine was initialized
    const searcher = indexer.searcher;
    const hasRelationshipEngine = searcher && searcher.relationshipEngine;
    
    console.log('\n🔗 Relationship Engine Status:');
    console.log(`   Searcher exists: ${searcher ? '✅' : '❌'}`);
    console.log(`   Relationship engine exists: ${hasRelationshipEngine ? '✅' : '❌'}`);
    
    // Get startup stage information
    const progress = stageTracker.getProgress();
    const stages = progress.stages;
    
    console.log('\n📊 Startup Stages Completed:');
    stages.forEach(stage => {
      const icon = stage.status === 'completed' ? '✅' : 
                   stage.status === 'in_progress' ? '🔄' : 
                   stage.status === 'failed' ? '❌' : '⏸️';
      console.log(`   ${icon} ${stage.name}: ${stage.status} ${stage.duration ? `(${stage.duration}ms)` : ''}`);
    });
    
    // Check specifically for relationship analysis stage
    const relationshipStage = stages.find(s => s.id === 'relationship_analysis' || s.name.includes('Relationship'));
    
    console.log('\n🎯 Key Validation Results:');
    console.log(`   Relationship stage found: ${relationshipStage ? '✅' : '❌'}`);
    console.log(`   Relationship stage completed: ${relationshipStage?.status === 'completed' ? '✅' : '❌'}`);
    console.log(`   Overall success: ${result.status === 'success' ? '✅' : '❌'}`);
    
    if (relationshipStage) {
      console.log(`   Relationship stage duration: ${relationshipStage.duration || 'unknown'}ms`);
      console.log(`   Relationship stage details: ${relationshipStage.details || 'none'}`);
    }
    
    // Success criteria
    const success = result.status === 'success' && 
                   hasRelationshipEngine &&
                   (relationshipStage?.status === 'completed' || relationshipStage?.status === 'in_progress');
    
    console.log(`\n🏆 Overall Test Result: ${success ? '✅ PASSED' : '❌ FAILED'}`);
    
    if (!success) {
      console.log('\n💡 Issues detected:');
      if (result.status !== 'success') console.log('   - Indexing failed');
      if (!hasRelationshipEngine) console.log('   - Relationship engine not initialized');
      if (!relationshipStage) console.log('   - Relationship analysis stage not found');
      if (relationshipStage && relationshipStage.status !== 'completed') {
        console.log(`   - Relationship stage not completed (status: ${relationshipStage.status})`);
      }
    }
    
    return success;
    
  } catch (error) {
    console.error('\n❌ Test failed with error:', error.message);
    console.error('Stack trace:', error.stack);
    return false;
  }
}

// Run test if this file is executed directly
if (require.main === module) {
  testRelationshipInitialization()
    .then(success => {
      process.exit(success ? 0 : 1);
    })
    .catch(error => {
      console.error('Test runner failed:', error);
      process.exit(1);
    });
}

module.exports = { testRelationshipInitialization };