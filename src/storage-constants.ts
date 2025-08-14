import * as path from 'path';
import * as crypto from 'crypto';
import * as os from 'os';

/**
 * Centralized storage constants and utilities for Cortex storage management
 */

// Storage file names
export const STORAGE_FILENAMES = {
  INDEX: 'index.json',
  RELATIONSHIPS: 'relationships.json',
  DELTAS: 'deltas',
  EMBEDDING_CACHE: 'embedding-cache.json'
} as const;

// Directory names
export const STORAGE_DIRECTORIES = {
  LOCAL_DEFAULT: '.cortex',
  GLOBAL_BASE: '.claude',
  GLOBAL_CORTEX: 'cortex-embeddings'
} as const;

// Storage paths utility class
export class StoragePaths {
  /**
   * Generate a consistent hash for repository identification
   * Matches the original format: repoName-16chars
   */
  static getRepositoryHash(repositoryPath: string): string {
    const absolutePath = path.resolve(repositoryPath);
    const hash = crypto.createHash('sha256').update(absolutePath).digest('hex');
    const repoName = path.basename(absolutePath);
    return `${repoName}-${hash.substring(0, 16)}`;
  }

  /**
   * Get the global storage base directory
   */
  static getGlobalStorageBase(): string {
    return path.join(os.homedir(), STORAGE_DIRECTORIES.GLOBAL_BASE, STORAGE_DIRECTORIES.GLOBAL_CORTEX);
  }

  /**
   * Get local storage paths for a repository
   */
  static getLocalPaths(repositoryPath: string, indexDir: string = STORAGE_DIRECTORIES.LOCAL_DEFAULT) {
    const localBasePath = path.join(repositoryPath, indexDir);
    
    return {
      basePath: localBasePath,
      indexPath: localBasePath,
      metadataPath: path.join(localBasePath, STORAGE_FILENAMES.INDEX),
      relationshipsPath: path.join(localBasePath, STORAGE_FILENAMES.RELATIONSHIPS),
      deltaPath: path.join(localBasePath, STORAGE_FILENAMES.DELTAS),
      embeddingCachePath: path.join(localBasePath, STORAGE_FILENAMES.EMBEDDING_CACHE)
    };
  }

  /**
   * Get global storage paths for a repository
   */
  static getGlobalPaths(repositoryPath: string) {
    const repoHash = this.getRepositoryHash(repositoryPath);
    const globalBasePath = path.join(this.getGlobalStorageBase(), repoHash);
    
    return {
      basePath: globalBasePath,
      indexPath: globalBasePath,
      metadataPath: path.join(globalBasePath, STORAGE_FILENAMES.INDEX),
      relationshipsPath: path.join(globalBasePath, STORAGE_FILENAMES.RELATIONSHIPS),
      deltaPath: path.join(globalBasePath, STORAGE_FILENAMES.DELTAS),
      embeddingCachePath: path.join(globalBasePath, STORAGE_FILENAMES.EMBEDDING_CACHE)
    };
  }

  /**
   * Get all storage paths (both local and global) for a repository
   */
  static getAllPaths(repositoryPath: string, indexDir: string = STORAGE_DIRECTORIES.LOCAL_DEFAULT) {
    return {
      local: this.getLocalPaths(repositoryPath, indexDir),
      global: this.getGlobalPaths(repositoryPath)
    };
  }
}