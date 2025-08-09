#!/usr/bin/env node

const { UnifiedStorageCoordinator } = require('../dist/unified-storage-coordinator');

class UnifiedStorageManager {
  constructor(repositoryPath = process.cwd()) {
    this.repositoryPath = repositoryPath;
  }

  async createCoordinator() {
    // Import from TypeScript compiled output
    const { UnifiedStorageCoordinator } = await import('../dist/unified-storage-coordinator.js');
    return new UnifiedStorageCoordinator(this.repositoryPath);
  }

  async getStatus() {
    console.log('📊 Unified Storage Status Report');
    console.log('================================');
    
    try {
      const coordinator = await this.createCoordinator();
      await coordinator.initialize();
      await coordinator.printStorageReport();
    } catch (error) {
      console.error('❌ Failed to generate status report:', error.message);
      process.exit(1);
    }
  }

  async validateConsistency() {
    console.log('🔍 Validating Storage Consistency');
    console.log('=================================');
    
    try {
      const coordinator = await this.createCoordinator();
      await coordinator.initialize();
      
      const validation = await coordinator.validateConsistency();
      
      console.log(`Status: ${validation.consistent ? '✅ Consistent' : '⚠️ Issues found'}`);
      
      if (validation.issues.length > 0) {
        console.log('\n❌ Issues:');
        validation.issues.forEach(issue => console.log(`   • ${issue}`));
      }
      
      if (validation.recommendations.length > 0) {
        console.log('\n💡 Recommendations:');
        validation.recommendations.forEach(rec => console.log(`   • ${rec}`));
      }
      
      if (!validation.consistent) {
        process.exit(1);
      }
    } catch (error) {
      console.error('❌ Failed to validate consistency:', error.message);
      process.exit(1);
    }
  }

  async syncAll() {
    console.log('🔄 Syncing All Storage Layers');
    console.log('=============================');
    
    try {
      const coordinator = await this.createCoordinator();
      await coordinator.initialize();
      await coordinator.syncAll();
      console.log('✅ All storage layers synchronized');
    } catch (error) {
      console.error('❌ Failed to sync storage:', error.message);
      process.exit(1);
    }
  }

  async clearAll() {
    console.log('🗑️ Clearing All Storage');
    console.log('=======================');
    
    const answer = await this.askConfirmation('This will delete all embeddings and relationship graphs. Continue? (yes/no): ');
    if (answer.toLowerCase() !== 'yes') {
      console.log('Operation cancelled');
      return;
    }
    
    try {
      const coordinator = await this.createCoordinator();
      await coordinator.initialize();
      await coordinator.clearAll();
      console.log('✅ All storage cleared');
    } catch (error) {
      console.error('❌ Failed to clear storage:', error.message);
      process.exit(1);
    }
  }

  async getStats() {
    console.log('📈 Unified Storage Statistics');
    console.log('=============================');
    
    try {
      const coordinator = await this.createCoordinator();
      await coordinator.initialize();
      
      const stats = await coordinator.getStorageStats();
      
      console.log('\n📊 Storage Overview:');
      console.log(`   Total Size: ${stats.totalSize}`);
      
      console.log('\n🔤 Embeddings:');
      console.log(`   Chunks: ${stats.embeddings.chunks}`);
      console.log(`   Files: ${stats.embeddings.files}`);
      console.log(`   Size: ${stats.embeddings.size}`);
      
      console.log('\n🔗 Relationships:');
      console.log(`   Symbols: ${stats.relationships.symbols}`);
      console.log(`   Relationships: ${stats.relationships.relationships}`);
      console.log(`   Size: ${stats.relationships.size}`);
      
    } catch (error) {
      console.error('❌ Failed to get statistics:', error.message);
      process.exit(1);
    }
  }

  askConfirmation(question) {
    return new Promise((resolve) => {
      const readline = require('readline');
      const rl = readline.createInterface({
        input: process.stdin,
        output: process.stdout
      });
      
      rl.question(question, (answer) => {
        rl.close();
        resolve(answer);
      });
    });
  }
}

// CLI interface
async function main() {
  const args = process.argv.slice(2);
  const command = args[0];
  const repoPath = args[1] || process.cwd();
  
  const manager = new UnifiedStorageManager(repoPath);
  
  try {
    switch (command) {
      case 'status':
        await manager.getStatus();
        break;
      case 'stats':
        await manager.getStats();
        break;
      case 'validate':
        await manager.validateConsistency();
        break;
      case 'sync':
        await manager.syncAll();
        break;
      case 'clear':
        await manager.clearAll();
        break;
      default:
        console.log('Usage: node manage-unified-storage.js <command> [repository-path]');
        console.log('');
        console.log('Commands:');
        console.log('  status      Show comprehensive storage status report');
        console.log('  stats       Show storage statistics');
        console.log('  validate    Validate storage consistency');
        console.log('  sync        Sync all storage layers');
        console.log('  clear       Clear all storage (interactive)');
        console.log('');
        console.log('Examples:');
        console.log('  npm run storage:status');
        console.log('  npm run storage:validate');
        console.log('  npm run storage:sync');
        console.log('  npm run storage:clear');
        process.exit(1);
    }
  } catch (error) {
    console.error('❌ Command failed:', error.message);
    process.exit(1);
  }
}

if (require.main === module) {
  main();
}

module.exports = { UnifiedStorageManager };