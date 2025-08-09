#!/usr/bin/env node

const fs = require('fs');
const path = require('path');
const os = require('os');
const crypto = require('crypto');

const CORTEX_DIR = '.cortex';
const INDEX_FILE = 'index.json';

class EmbeddingManager {
  constructor(repoPath = process.cwd()) {
    this.repoPath = repoPath;
    
    // Local storage
    this.cortexPath = path.join(repoPath, CORTEX_DIR);
    this.indexPath = path.join(this.cortexPath, INDEX_FILE);
    
    // Global storage
    const repoHash = this.getRepositoryHash(repoPath);
    const claudeDir = path.join(os.homedir(), '.claude', 'cortex-embeddings');
    this.globalCortexPath = path.join(claudeDir, repoHash);
    this.globalIndexPath = path.join(this.globalCortexPath, INDEX_FILE);
  }

  getRepositoryHash(repoPath) {
    const absolutePath = path.resolve(repoPath);
    const hash = crypto.createHash('sha256').update(absolutePath).digest('hex');
    const repoName = path.basename(absolutePath);
    return `${repoName}-${hash.substring(0, 16)}`;
  }

  async getStats() {
    try {
      const localExists = fs.existsSync(this.indexPath);
      const globalExists = fs.existsSync(this.globalIndexPath);
      
      if (!localExists && !globalExists) {
        return {
          exists: false,
          message: 'No embedding index found in local or global storage'
        };
      }

      const results = {
        local: { exists: localExists },
        global: { exists: globalExists }
      };

      if (localExists) {
        const stats = fs.statSync(this.indexPath);
        const indexData = JSON.parse(fs.readFileSync(this.indexPath, 'utf-8'));
        const cortexDirStats = this.getDirSize(this.cortexPath);

        results.local = {
          exists: true,
          totalChunks: indexData.chunks.length,
          totalFiles: Object.keys(indexData.fileHashes || {}).length,
          lastUpdated: new Date(indexData.timestamp).toLocaleString(),
          indexSizeMB: (stats.size / 1024 / 1024).toFixed(2),
          totalSizeMB: (cortexDirStats / 1024 / 1024).toFixed(2),
          embeddingModel: indexData.metadata?.embeddingModel || 'Unknown',
          version: indexData.version || 'Unknown',
          path: this.indexPath
        };
      }

      if (globalExists) {
        const stats = fs.statSync(this.globalIndexPath);
        const indexData = JSON.parse(fs.readFileSync(this.globalIndexPath, 'utf-8'));
        const cortexDirStats = this.getDirSize(this.globalCortexPath);

        results.global = {
          exists: true,
          totalChunks: indexData.chunks.length,
          totalFiles: Object.keys(indexData.fileHashes || {}).length,
          lastUpdated: new Date(indexData.timestamp).toLocaleString(),
          indexSizeMB: (stats.size / 1024 / 1024).toFixed(2),
          totalSizeMB: (cortexDirStats / 1024 / 1024).toFixed(2),
          embeddingModel: indexData.metadata?.embeddingModel || 'Unknown',
          version: indexData.version || 'Unknown',
          path: this.globalIndexPath
        };
      }

      return results;
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

  async syncToGlobal() {
    try {
      if (!fs.existsSync(this.indexPath)) {
        return { success: false, message: 'No local cache to sync' };
      }

      // Ensure global directory exists
      if (!fs.existsSync(this.globalCortexPath)) {
        fs.mkdirSync(this.globalCortexPath, { recursive: true });
      }

      const indexData = fs.readFileSync(this.indexPath, 'utf-8');
      fs.writeFileSync(this.globalIndexPath, indexData);

      return {
        success: true,
        message: `Synced to global storage: ${this.globalIndexPath}`
      };
    } catch (error) {
      return {
        success: false,
        error: error.message
      };
    }
  }

  async syncToLocal() {
    try {
      if (!fs.existsSync(this.globalIndexPath)) {
        return { success: false, message: 'No global cache to sync' };
      }

      // Ensure local directory exists
      if (!fs.existsSync(this.cortexPath)) {
        fs.mkdirSync(this.cortexPath, { recursive: true });
      }

      const indexData = fs.readFileSync(this.globalIndexPath, 'utf-8');
      fs.writeFileSync(this.indexPath, indexData);

      return {
        success: true,
        message: `Synced to local storage: ${this.indexPath}`
      };
    } catch (error) {
      return {
        success: false,
        error: error.message
      };
    }
  }

  async getStorageInfo() {
    const localExists = fs.existsSync(this.indexPath);
    const globalExists = fs.existsSync(this.globalIndexPath);

    const info = {
      local: { exists: localExists, path: this.indexPath },
      global: { exists: globalExists, path: this.globalIndexPath }
    };

    if (localExists) {
      const stats = fs.statSync(this.indexPath);
      info.local.lastModified = stats.mtime;
    }

    if (globalExists) {
      const stats = fs.statSync(this.globalIndexPath);
      info.global.lastModified = stats.mtime;
    }

    return info;
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
      if (stats.local?.exists || stats.global?.exists) {
        console.log('📊 Embedding Cache Statistics:');
        
        if (stats.local?.exists) {
          console.log('\n📁 Local Storage (.cortex/):');
          console.log(`   Total Chunks: ${stats.local.totalChunks.toLocaleString()}`);
          console.log(`   Total Files: ${stats.local.totalFiles.toLocaleString()}`);
          console.log(`   Last Updated: ${stats.local.lastUpdated}`);
          console.log(`   Index Size: ${stats.local.indexSizeMB} MB`);
          console.log(`   Total Size: ${stats.local.totalSizeMB} MB`);
          console.log(`   Model: ${stats.local.embeddingModel}`);
          console.log(`   Path: ${stats.local.path}`);
        }
        
        if (stats.global?.exists) {
          console.log('\n🌐 Global Storage (~/.claude/):');
          console.log(`   Total Chunks: ${stats.global.totalChunks.toLocaleString()}`);
          console.log(`   Total Files: ${stats.global.totalFiles.toLocaleString()}`);
          console.log(`   Last Updated: ${stats.global.lastUpdated}`);
          console.log(`   Index Size: ${stats.global.indexSizeMB} MB`);
          console.log(`   Total Size: ${stats.global.totalSizeMB} MB`);
          console.log(`   Model: ${stats.global.embeddingModel}`);
          console.log(`   Path: ${stats.global.path}`);
        }
      } else {
        console.log('❌ No embedding cache found in local or global storage');
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

    case 'sync-to-global':
    case 'sync-global':
      const syncGlobalResult = await manager.syncToGlobal();
      if (syncGlobalResult.success) {
        console.log(`✅ ${syncGlobalResult.message}`);
      } else {
        console.error(`❌ ${syncGlobalResult.error || syncGlobalResult.message}`);
        process.exit(1);
      }
      break;

    case 'sync-to-local':
    case 'sync-local':
      const syncLocalResult = await manager.syncToLocal();
      if (syncLocalResult.success) {
        console.log(`✅ ${syncLocalResult.message}`);
      } else {
        console.error(`❌ ${syncLocalResult.error || syncLocalResult.message}`);
        process.exit(1);
      }
      break;

    case 'info':
    case 'storage-info':
      const storageInfo = await manager.getStorageInfo();
      console.log('🗄️ Storage Information:');
      console.log(`📁 Local: ${storageInfo.local.exists ? '✅ Exists' : '❌ Missing'}`);
      console.log(`   Path: ${storageInfo.local.path}`);
      if (storageInfo.local.lastModified) {
        console.log(`   Modified: ${storageInfo.local.lastModified.toLocaleString()}`);
      }
      console.log(`🌐 Global: ${storageInfo.global.exists ? '✅ Exists' : '❌ Missing'}`);
      console.log(`   Path: ${storageInfo.global.path}`);
      if (storageInfo.global.lastModified) {
        console.log(`   Modified: ${storageInfo.global.lastModified.toLocaleString()}`);
      }
      break;

    case 'help':
    default:
      console.log('🔧 Cortex Embedding Cache Manager');
      console.log('');
      console.log('Usage: node manage-embeddings.js <command>');
      console.log('');
      console.log('Commands:');
      console.log('  stats          Show cache statistics for both local and global storage');
      console.log('  clear          Clear the embedding cache (both local and global)');
      console.log('  validate       Validate index integrity');
      console.log('  backup         Create a backup of the cache');
      console.log('  restore        Restore from backup file');
      console.log('  sync-to-global Sync local cache to global storage (~/.claude)');
      console.log('  sync-to-local  Sync global cache to local storage (.cortex)');
      console.log('  info           Show storage paths and modification times');
      console.log('  help           Show this help message');
      console.log('');
      console.log('Dual Storage System:');
      console.log('  📁 Local:  .cortex/ (fast access, stays with repo)');
      console.log('  🌐 Global: ~/.claude/cortex-embeddings/ (synced across dev environments)');
      console.log('');
      console.log('Examples:');
      console.log('  node manage-embeddings.js stats');
      console.log('  node manage-embeddings.js info');
      console.log('  node manage-embeddings.js sync-to-global');
      console.log('  node manage-embeddings.js clear');
      console.log('  node manage-embeddings.js backup');
      break;
  }
}

if (require.main === module) {
  main().catch(console.error);
}

module.exports = { EmbeddingManager };