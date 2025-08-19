import * as path from 'path';
import * as crypto from 'crypto';
import * as os from 'os';
import * as fs from 'fs/promises';
import * as zlib from 'zlib';
import { promisify } from 'util';

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

// Compression configuration
export const COMPRESSION_CONFIG = {
  SIZE_THRESHOLD: 10 * 1024 * 1024, // 10MB - files larger than this get compressed
  COMPRESSION_EXTENSION: '.gz',
  COMPRESSION_LEVEL: 6 // Good balance of speed vs compression ratio
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

  /**
   * Get compressed version of a file path
   */
  static getCompressedPath(jsonPath: string): string {
    return jsonPath + COMPRESSION_CONFIG.COMPRESSION_EXTENSION;
  }

  /**
   * Get uncompressed version of a file path
   */
  static getUncompressedPath(gzPath: string): string {
    return gzPath.endsWith(COMPRESSION_CONFIG.COMPRESSION_EXTENSION) 
      ? gzPath.slice(0, -COMPRESSION_CONFIG.COMPRESSION_EXTENSION.length)
      : gzPath;
  }

  /**
   * Check if a file path represents a compressed file
   */
  static isCompressedPath(filePath: string): boolean {
    return filePath.endsWith(COMPRESSION_CONFIG.COMPRESSION_EXTENSION);
  }
}

// Compression utilities
const gzipAsync = promisify(zlib.gzip);
const gunzipAsync = promisify(zlib.gunzip);

export class CompressionUtils {
  /**
   * Read a file with automatic decompression support
   * Tries compressed version first, falls back to uncompressed
   */
  static async readFileWithDecompression(filePath: string): Promise<string> {
    const compressedPath = StoragePaths.getCompressedPath(filePath);
    
    // Try compressed version first
    try {
      const compressedData = await fs.readFile(compressedPath);
      const decompressed = await gunzipAsync(compressedData);
      return decompressed.toString('utf-8');
    } catch (error) {
      // Fall back to uncompressed version
      try {
        return await fs.readFile(filePath, 'utf-8');
      } catch (fallbackError) {
        // If neither exists, throw the original error for the expected file
        throw new Error(`Could not read file ${filePath} (compressed or uncompressed): ${error}`);
      }
    }
  }

  /**
   * Write a file with automatic compression for large files
   * Compresses if file size exceeds threshold
   */
  static async writeFileWithCompression(filePath: string, data: string): Promise<void> {
    const dataSize = Buffer.byteLength(data, 'utf-8');
    
    if (dataSize > COMPRESSION_CONFIG.SIZE_THRESHOLD) {
      // Compress and save as .gz file
      const compressed = await gzipAsync(data, { level: COMPRESSION_CONFIG.COMPRESSION_LEVEL });
      const compressedPath = StoragePaths.getCompressedPath(filePath);
      
      // Ensure directory exists
      await fs.mkdir(path.dirname(compressedPath), { recursive: true });
      await fs.writeFile(compressedPath, compressed);
      
      // Remove uncompressed version if it exists
      try {
        await fs.unlink(filePath);
      } catch (error) {
        // Ignore if file doesn't exist
      }
    } else {
      // Save as regular uncompressed file
      await fs.mkdir(path.dirname(filePath), { recursive: true });
      await fs.writeFile(filePath, data, 'utf-8');
      
      // Remove compressed version if it exists
      const compressedPath = StoragePaths.getCompressedPath(filePath);
      try {
        await fs.unlink(compressedPath);
      } catch (error) {
        // Ignore if file doesn't exist
      }
    }
  }

  /**
   * Get file size (uncompressed) for a path that might be compressed
   */
  static async getUncompressedSize(filePath: string): Promise<number> {
    const compressedPath = StoragePaths.getCompressedPath(filePath);
    
    // Check compressed version first
    try {
      const compressedData = await fs.readFile(compressedPath);
      const decompressed = await gunzipAsync(compressedData);
      return decompressed.length;
    } catch (error) {
      // Fall back to uncompressed version
      try {
        const stats = await fs.stat(filePath);
        return stats.size;
      } catch (fallbackError) {
        return 0; // File doesn't exist
      }
    }
  }

  /**
   * Check if a file exists (compressed or uncompressed)
   */
  static async fileExists(filePath: string): Promise<boolean> {
    const compressedPath = StoragePaths.getCompressedPath(filePath);
    
    try {
      await fs.access(compressedPath);
      return true;
    } catch (error) {
      try {
        await fs.access(filePath);
        return true;
      } catch (fallbackError) {
        return false;
      }
    }
  }
}