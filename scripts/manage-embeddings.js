#!/usr/bin/env node

const fs = require('fs');
const path = require('path');

const CORTEX_DIR = '.cortex';
const INDEX_FILE = 'index.json';

class EmbeddingManager {
  constructor(repoPath = process.cwd()) {
    this.repoPath = repoPath;
    this.cortexPath = path.join(repoPath, CORTEX_DIR);
    this.indexPath = path.join(this.cortexPath, INDEX_FILE);
  }

  async getStats() {
    try {
      if (!fs.existsSync(this.indexPath)) {
        return {
          exists: false,
          message: 'No embedding index found'
        };
      }

      const stats = fs.statSync(this.indexPath);
      const indexData = JSON.parse(fs.readFileSync(this.indexPath, 'utf-8'));
      
      const cortexDirStats = this.getDirSize(this.cortexPath);

      return {
        exists: true,
        totalChunks: indexData.chunks.length,
        totalFiles: Object.keys(indexData.fileHashes || {}).length,
        lastUpdated: new Date(indexData.timestamp).toLocaleString(),
        indexSizeMB: (stats.size / 1024 / 1024).toFixed(2),
        totalSizeMB: (cortexDirStats / 1024 / 1024).toFixed(2),
        embeddingModel: indexData.metadata?.embeddingModel || 'Unknown',
        version: indexData.version || 'Unknown'
      };
    } catch (error) {
      return {
        exists: false,
        error: error.message
      };
    }
  }

  getDirSize(dirPath) {
    let totalSize = 0;
    
    try {
      const files = fs.readdirSync(dirPath);
      
      for (const file of files) {
        const filePath = path.join(dirPath, file);
        const stats = fs.statSync(filePath);
        
        if (stats.isDirectory()) {
          totalSize += this.getDirSize(filePath);
        } else {
          totalSize += stats.size;
        }
      }
    } catch (error) {
      // Directory doesn't exist or no access
    }
    
    return totalSize;
  }

  async clearCache() {
    try {
      if (fs.existsSync(this.cortexPath)) {
        fs.rmSync(this.cortexPath, { recursive: true, force: true });
        return { success: true, message: 'Embedding cache cleared successfully' };
      } else {
        return { success: true, message: 'No cache to clear' };
      }
    } catch (error) {
      return { success: false, error: error.message };
    }
  }

  async validateIndex() {
    try {
      if (!fs.existsSync(this.indexPath)) {
        return { valid: false, message: 'Index file does not exist' };
      }

      const indexData = JSON.parse(fs.readFileSync(this.indexPath, 'utf-8'));
      
      // Basic validation
      const issues = [];
      
      if (!indexData.version) issues.push('Missing version');
      if (!indexData.chunks || !Array.isArray(indexData.chunks)) issues.push('Invalid chunks data');
      if (!indexData.metadata) issues.push('Missing metadata');
      if (!indexData.fileHashes) issues.push('Missing file hashes');
      
      // Check chunk integrity
      let validChunks = 0;
      let invalidChunks = 0;
      
      for (const chunk of indexData.chunks || []) {
        if (chunk.chunk_id && chunk.file_path && chunk.content && Array.isArray(chunk.embedding)) {
          validChunks++;
        } else {
          invalidChunks++;
        }
      }

      return {
        valid: issues.length === 0 && invalidChunks === 0,
        issues,
        stats: {
          validChunks,
          invalidChunks,
          totalChunks: (indexData.chunks || []).length
        }
      };
    } catch (error) {
      return {
        valid: false,
        error: error.message
      };
    }
  }

  async backup() {
    try {
      if (!fs.existsSync(this.cortexPath)) {
        return { success: false, message: 'No cache to backup' };
      }

      const timestamp = new Date().toISOString().replace(/[:.]/g, '-');
      const backupPath = path.join(this.repoPath, `cortex-backup-${timestamp}.tar.gz`);
      
      const { execSync } = require('child_process');
      execSync(`tar -czf "${backupPath}" -C "${this.repoPath}" "${CORTEX_DIR}"`);
      
      return {
        success: true,
        message: `Backup created: ${backupPath}`,
        backupPath
      };
    } catch (error) {
      return {
        success: false,
        error: error.message
      };
    }
  }

  async restore(backupPath) {
    try {
      if (!fs.existsSync(backupPath)) {
        return { success: false, message: 'Backup file does not exist' };
      }

      // Clear existing cache
      if (fs.existsSync(this.cortexPath)) {
        fs.rmSync(this.cortexPath, { recursive: true, force: true });
      }

      const { execSync } = require('child_process');
      execSync(`tar -xzf "${backupPath}" -C "${this.repoPath}"`);
      
      return {
        success: true,
        message: `Cache restored from: ${backupPath}`
      };
    } catch (error) {
      return {
        success: false,
        error: error.message
      };
    }
  }
}

// CLI Interface
async function main() {
  const command = process.argv[2];
  const manager = new EmbeddingManager();

  switch (command) {
    case 'stats':
    case 'status':
      const stats = await manager.getStats();
      if (stats.exists) {
        console.log('📊 Embedding Cache Statistics:');
        console.log(`   Total Chunks: ${stats.totalChunks.toLocaleString()}`);
        console.log(`   Total Files: ${stats.totalFiles.toLocaleString()}`);
        console.log(`   Last Updated: ${stats.lastUpdated}`);
        console.log(`   Index Size: ${stats.indexSizeMB} MB`);
        console.log(`   Total Size: ${stats.totalSizeMB} MB`);
        console.log(`   Model: ${stats.embeddingModel}`);
        console.log(`   Version: ${stats.version}`);
      } else {
        console.log('❌ No embedding cache found');
        if (stats.error) console.log(`   Error: ${stats.error}`);
      }
      break;

    case 'clear':
    case 'clean':
      const clearResult = await manager.clearCache();
      if (clearResult.success) {
        console.log(`✅ ${clearResult.message}`);
      } else {
        console.error(`❌ ${clearResult.error}`);
        process.exit(1);
      }
      break;

    case 'validate':
      const validation = await manager.validateIndex();
      if (validation.valid) {
        console.log('✅ Embedding index is valid');
        console.log(`   Valid chunks: ${validation.stats.validChunks}`);
        console.log(`   Total chunks: ${validation.stats.totalChunks}`);
      } else {
        console.log('❌ Embedding index has issues:');
        if (validation.issues) {
          validation.issues.forEach(issue => console.log(`   - ${issue}`));
        }
        if (validation.stats) {
          console.log(`   Valid chunks: ${validation.stats.validChunks}`);
          console.log(`   Invalid chunks: ${validation.stats.invalidChunks}`);
        }
        if (validation.error) {
          console.log(`   Error: ${validation.error}`);
        }
      }
      break;

    case 'backup':
      const backupResult = await manager.backup();
      if (backupResult.success) {
        console.log(`✅ ${backupResult.message}`);
      } else {
        console.error(`❌ ${backupResult.error}`);
        process.exit(1);
      }
      break;

    case 'restore':
      const backupPath = process.argv[3];
      if (!backupPath) {
        console.error('❌ Please provide backup file path');
        console.log('Usage: node manage-embeddings.js restore <backup-file>');
        process.exit(1);
      }
      
      const restoreResult = await manager.restore(backupPath);
      if (restoreResult.success) {
        console.log(`✅ ${restoreResult.message}`);
      } else {
        console.error(`❌ ${restoreResult.error}`);
        process.exit(1);
      }
      break;

    case 'help':
    default:
      console.log('🔧 Cortex Embedding Cache Manager');
      console.log('');
      console.log('Usage: node manage-embeddings.js <command>');
      console.log('');
      console.log('Commands:');
      console.log('  stats     Show cache statistics');
      console.log('  clear     Clear the embedding cache');
      console.log('  validate  Validate index integrity');
      console.log('  backup    Create a backup of the cache');
      console.log('  restore   Restore from backup file');
      console.log('  help      Show this help message');
      console.log('');
      console.log('Examples:');
      console.log('  node manage-embeddings.js stats');
      console.log('  node manage-embeddings.js clear');
      console.log('  node manage-embeddings.js backup');
      console.log('  node manage-embeddings.js restore cortex-backup-*.tar.gz');
      break;
  }
}

if (require.main === module) {
  main().catch(console.error);
}

module.exports = { EmbeddingManager };