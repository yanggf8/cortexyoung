import * as fs from 'fs/promises';
import * as path from 'path';
import { log as timestampedLog, warn as timestampedWarn, error as timestampedError } from './logging-utils';

export interface BackupValidationResult {
  isValid: boolean;
  chunkCount: number;
  hasValidMetadata: boolean;
  timestamp?: number;
  reason?: string;
}

export interface BackupResult {
  success: boolean;
  backupPath?: string;
  skipReason?: string;
  validationResult: BackupValidationResult;
}

export class EmbeddingBackupUtility {
  /**
   * Validates embedding data before backup
   */
  static async validateEmbeddingData(indexPath: string): Promise<BackupValidationResult> {
    try {
      // Check if file exists
      await fs.access(indexPath);
      
      // Read and parse JSON
      const content = await fs.readFile(indexPath, 'utf-8');
      const data = JSON.parse(content);
      
      // Validate structure and content
      const chunkCount = data.chunks?.length || 0;
      const hasValidMetadata = !!(data.metadata && data.timestamp);
      
      if (chunkCount === 0) {
        return {
          isValid: false,
          chunkCount: 0,
          hasValidMetadata,
          reason: '❌ No chunks found - empty embedding data'
        };
      }
      
      if (!hasValidMetadata) {
        return {
          isValid: false,
          chunkCount,
          hasValidMetadata: false,
          reason: '❌ Missing essential metadata'
        };
      }
      
      return {
        isValid: true,
        chunkCount,
        hasValidMetadata: true,
        timestamp: data.timestamp
      };
      
    } catch (error) {
      if (error instanceof SyntaxError) {
        return {
          isValid: false,
          chunkCount: 0,
          hasValidMetadata: false,
          reason: '❌ Corrupted JSON (likely merge conflicts)'
        };
      }
      
      return {
        isValid: false,
        chunkCount: 0,
        hasValidMetadata: false,
        reason: `❌ Read error: ${error instanceof Error ? error.message : 'Unknown error'}`
      };
    }
  }
  
  /**
   * Creates backup only if data is valid and worth preserving
   */
  static async createValidatedBackup(
    sourceDir: string,
    reason: string = 'pre-rebuild-safety'
  ): Promise<BackupResult> {
    timestampedLog(`🔍 Validating embedding data before backup...`);
    
    // Find the main index file
    const indexPath = path.join(sourceDir, 'index.json');
    const validation = await this.validateEmbeddingData(indexPath);
    
    if (!validation.isValid) {
      timestampedWarn(`⚠️  Skipping backup: ${validation.reason}`);
      return {
        success: false,
        skipReason: validation.reason,
        validationResult: validation
      };
    }
    
    timestampedLog(`✅ Valid embedding data found: ${validation.chunkCount} chunks`);
    timestampedLog(`🔄 Creating pre-rebuild backup...`);
    
    // Create timestamped backup directory
    const timestamp = new Date().toISOString().replace(/[:.]/g, '-');
    const backupDir = `embedding-backup-${reason}-${timestamp}`;
    
    try {
      // Create backup directory
      await fs.mkdir(backupDir, { recursive: true });
      
      // Copy the entire source directory
      await this.copyDirectory(sourceDir, path.join(backupDir, 'cortex-embeddings'));
      
      // Create backup metadata
      const metadata = {
        created: new Date().toISOString(),
        reason,
        sourceDirectory: sourceDir,
        validation: {
          chunkCount: validation.chunkCount,
          hasValidMetadata: validation.hasValidMetadata,
          timestamp: validation.timestamp
        },
        backupDirectory: backupDir
      };
      
      await fs.writeFile(
        path.join(backupDir, 'backup-metadata.json'),
        JSON.stringify(metadata, null, 2)
      );
      
      timestampedLog(`✅ Backup created: ${backupDir}`);
      timestampedLog(`📊 Preserved ${validation.chunkCount} chunks safely`);
      
      return {
        success: true,
        backupPath: backupDir,
        validationResult: validation
      };
      
    } catch (error) {
      timestampedError(`❌ Backup creation failed: ${error instanceof Error ? error.message : 'Unknown error'}`);
      return {
        success: false,
        skipReason: `Backup creation failed: ${error instanceof Error ? error.message : 'Unknown error'}`,
        validationResult: validation
      };
    }
  }
  
  /**
   * Helper to recursively copy directories
   */
  private static async copyDirectory(source: string, destination: string): Promise<void> {
    await fs.mkdir(destination, { recursive: true });
    
    const items = await fs.readdir(source, { withFileTypes: true });
    
    for (const item of items) {
      const sourcePath = path.join(source, item.name);
      const destPath = path.join(destination, item.name);
      
      if (item.isDirectory()) {
        await this.copyDirectory(sourcePath, destPath);
      } else {
        await fs.copyFile(sourcePath, destPath);
      }
    }
  }
  
  /**
   * Quick validation check - returns true only if data is worth backing up
   */
  static async hasValidEmbeddingData(indexPath: string): Promise<boolean> {
    const validation = await this.validateEmbeddingData(indexPath);
    return validation.isValid && validation.chunkCount > 0;
  }
}