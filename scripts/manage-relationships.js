#!/usr/bin/env node

const fs = require('fs').promises;
const path = require('path');
const crypto = require('crypto');
const os = require('os');

class RelationshipGraphManager {
  constructor(repositoryPath = process.cwd()) {
    this.repositoryPath = repositoryPath;
    
    // Local storage (in repo)
    this.localGraphPath = path.join(repositoryPath, '.cortex');
    this.metadataPath = path.join(this.localGraphPath, 'relationships.json');
    
    // Global storage (in ~/.claude)
    const repoHash = this.getRepositoryHash(repositoryPath);
    const claudeDir = path.join(os.homedir(), '.claude', 'cortex-embeddings');
    this.globalGraphPath = path.join(claudeDir, repoHash);
    this.globalMetadataPath = path.join(this.globalGraphPath, 'relationships.json');
  }

  getRepositoryHash(repoPath) {
    const absolutePath = path.resolve(repoPath);
    const hash = crypto.createHash('sha256').update(absolutePath).digest('hex');
    const repoName = path.basename(absolutePath);
    return `${repoName}-${hash.substring(0, 16)}`;
  }

  async relationshipGraphExists() {
    try {
      await fs.access(this.metadataPath);
      return true;
    } catch {
      return false;
    }
  }

  async globalRelationshipGraphExists() {
    try {
      await fs.access(this.globalMetadataPath);
      return true;
    } catch {
      return false;
    }
  }

  async getStats() {
    console.log('📊 Relationship Graph Statistics');
    console.log('=================================');
    
    const localExists = await this.relationshipGraphExists();
    const globalExists = await this.globalRelationshipGraphExists();
    
    console.log(`📁 Local storage: ${localExists ? 'EXISTS' : 'MISSING'}`);
    if (localExists) {
      try {
        const stats = await fs.stat(this.metadataPath);
        const data = JSON.parse(await fs.readFile(this.metadataPath, 'utf-8'));
        console.log(`   Path: ${this.metadataPath}`);
        console.log(`   Size: ${(stats.size / 1024).toFixed(2)} KB`);
        console.log(`   Last modified: ${stats.mtime.toISOString()}`);
        console.log(`   Symbols: ${data.metadata?.totalSymbols || 'unknown'}`);
        console.log(`   Relationships: ${data.metadata?.totalRelationships || 'unknown'}`);
        console.log(`   Schema version: ${data.schemaVersion || 'unknown'}`);
      } catch (error) {
        console.log(`   Error: ${error.message}`);
      }
    }
    
    console.log(`🌐 Global storage: ${globalExists ? 'EXISTS' : 'MISSING'}`);
    if (globalExists) {
      try {
        const stats = await fs.stat(this.globalMetadataPath);
        const data = JSON.parse(await fs.readFile(this.globalMetadataPath, 'utf-8'));
        console.log(`   Path: ${this.globalMetadataPath}`);
        console.log(`   Size: ${(stats.size / 1024).toFixed(2)} KB`);
        console.log(`   Last modified: ${stats.mtime.toISOString()}`);
        console.log(`   Symbols: ${data.metadata?.totalSymbols || 'unknown'}`);
        console.log(`   Relationships: ${data.metadata?.totalRelationships || 'unknown'}`);
        console.log(`   Schema version: ${data.schemaVersion || 'unknown'}`);
      } catch (error) {
        console.log(`   Error: ${error.message}`);
      }
    }
    
    if (!localExists && !globalExists) {
      console.log('⚠️  No relationship graphs found. Run the server once to generate them.');
    }
  }

  async clearRelationshipGraphs() {
    console.log('🗑️  Clearing relationship graphs...');
    
    try {
      // Clear local storage
      if (await this.relationshipGraphExists()) {
        await fs.rm(this.localGraphPath, { recursive: true, force: true });
        await fs.mkdir(this.localGraphPath, { recursive: true });
        console.log('✅ Cleared local relationship graph');
      }
      
      // Clear global storage
      if (await this.globalRelationshipGraphExists()) {
        await fs.rm(this.globalGraphPath, { recursive: true, force: true });
        console.log('✅ Cleared global relationship graph');
      }
      
      console.log('🎉 All relationship graphs cleared successfully');
    } catch (error) {
      console.error('❌ Failed to clear relationship graphs:', error.message);
      process.exit(1);
    }
  }

  async syncToGlobal() {
    console.log('🔄 Syncing local relationship graph to global storage...');
    
    if (!(await this.relationshipGraphExists())) {
      console.log('⚠️  No local relationship graph found');
      return;
    }
    
    try {
      // Ensure global directory exists
      await fs.mkdir(this.globalGraphPath, { recursive: true });
      
      // Copy relationship graph
      const graphData = await fs.readFile(this.metadataPath, 'utf-8');
      const tempPath = this.globalMetadataPath + '.tmp';
      await fs.writeFile(tempPath, graphData);
      await fs.rename(tempPath, this.globalMetadataPath);
      
      console.log('✅ Synced relationship graph to global storage');
      console.log(`📁 Local: ${this.metadataPath}`);
      console.log(`🌐 Global: ${this.globalMetadataPath}`);
    } catch (error) {
      console.error('❌ Failed to sync to global storage:', error.message);
      process.exit(1);
    }
  }

  async syncToLocal() {
    console.log('🔄 Syncing global relationship graph to local storage...');
    
    if (!(await this.globalRelationshipGraphExists())) {
      console.log('⚠️  No global relationship graph found');
      return;
    }
    
    try {
      // Check if local is outdated
      const localExists = await this.relationshipGraphExists();
      let shouldSync = true;
      
      if (localExists) {
        const [localStats, globalStats] = await Promise.all([
          fs.stat(this.metadataPath),
          fs.stat(this.globalMetadataPath)
        ]);
        shouldSync = globalStats.mtime > localStats.mtime;
      }
      
      if (!shouldSync) {
        console.log('📋 Local relationship graph is already up to date');
        return;
      }
      
      // Ensure local directory exists
      await fs.mkdir(this.localGraphPath, { recursive: true });
      
      // Copy relationship graph
      const graphData = await fs.readFile(this.globalMetadataPath, 'utf-8');
      const tempPath = this.metadataPath + '.tmp';
      await fs.writeFile(tempPath, graphData);
      await fs.rename(tempPath, this.metadataPath);
      
      console.log('✅ Synced relationship graph to local storage');
      console.log(`🌐 Global: ${this.globalMetadataPath}`);
      console.log(`📁 Local: ${this.metadataPath}`);
    } catch (error) {
      console.error('❌ Failed to sync to local storage:', error.message);
      process.exit(1);
    }
  }

  async getStorageInfo() {
    console.log('📁 Relationship Graph Storage Information');
    console.log('========================================');
    
    const [localExists, globalExists] = await Promise.all([
      this.relationshipGraphExists(),
      this.globalRelationshipGraphExists()
    ]);

    console.log('Local Storage:');
    console.log(`  Path: ${this.metadataPath}`);
    console.log(`  Exists: ${localExists ? 'YES' : 'NO'}`);
    
    if (localExists) {
      try {
        const stats = await fs.stat(this.metadataPath);
        console.log(`  Last Modified: ${stats.mtime.toISOString()}`);
        console.log(`  Size: ${(stats.size / 1024).toFixed(2)} KB`);
      } catch (error) {
        console.log(`  Error: ${error.message}`);
      }
    }

    console.log('\nGlobal Storage:');
    console.log(`  Path: ${this.globalMetadataPath}`);
    console.log(`  Exists: ${globalExists ? 'YES' : 'NO'}`);
    
    if (globalExists) {
      try {
        const stats = await fs.stat(this.globalMetadataPath);
        console.log(`  Last Modified: ${stats.mtime.toISOString()}`);
        console.log(`  Size: ${(stats.size / 1024).toFixed(2)} KB`);
      } catch (error) {
        console.log(`  Error: ${error.message}`);
      }
    }

    // Sync recommendations
    console.log('\nRecommendations:');
    if (localExists && globalExists) {
      try {
        const [localStats, globalStats] = await Promise.all([
          fs.stat(this.metadataPath),
          fs.stat(this.globalMetadataPath)
        ]);
        
        if (localStats.mtime > globalStats.mtime) {
          console.log('💡 Local is newer - consider running: npm run relationships:sync-global');
        } else if (globalStats.mtime > localStats.mtime) {
          console.log('💡 Global is newer - consider running: npm run relationships:sync-local');
        } else {
          console.log('✅ Both storages are in sync');
        }
      } catch (error) {
        console.log(`⚠️  Could not compare timestamps: ${error.message}`);
      }
    } else if (localExists && !globalExists) {
      console.log('💡 Only local exists - consider running: npm run relationships:sync-global');
    } else if (!localExists && globalExists) {
      console.log('💡 Only global exists - consider running: npm run relationships:sync-local');
    } else {
      console.log('💡 No relationship graphs found - run the server to generate them');
    }
  }
}

// CLI interface
async function main() {
  const args = process.argv.slice(2);
  const command = args[0];
  const repoPath = args[1] || process.cwd();
  
  const manager = new RelationshipGraphManager(repoPath);
  
  try {
    switch (command) {
      case 'stats':
        await manager.getStats();
        break;
      case 'clear':
        await manager.clearRelationshipGraphs();
        break;
      case 'sync-to-global':
        await manager.syncToGlobal();
        break;
      case 'sync-to-local':
        await manager.syncToLocal();
        break;
      case 'info':
        await manager.getStorageInfo();
        break;
      default:
        console.log('Usage: node manage-relationships.js <command> [repository-path]');
        console.log('');
        console.log('Commands:');
        console.log('  stats           Show relationship graph statistics');
        console.log('  clear           Clear all relationship graphs');
        console.log('  sync-to-global  Sync local graph to global storage');
        console.log('  sync-to-local   Sync global graph to local storage');
        console.log('  info            Show storage paths and sync status');
        console.log('');
        console.log('Examples:');
        console.log('  npm run relationships:stats');
        console.log('  npm run relationships:clear');
        console.log('  npm run relationships:sync-global');
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

module.exports = { RelationshipGraphManager };