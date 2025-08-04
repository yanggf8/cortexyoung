import { CORTEX_SCHEMA_VERSION, SUPPORTED_SCHEMA_VERSIONS, SchemaInfo } from './types';
import * as semver from 'semver';

export class SchemaValidator {
  
  /**
   * Validates schema compatibility and determines if migration is needed
   */
  static validateSchema(indexVersion: string): SchemaInfo {
    const currentVersion = CORTEX_SCHEMA_VERSION;
    
    // Check if version is supported
    const compatible = SUPPORTED_SCHEMA_VERSIONS.includes(indexVersion);
    
    // Determine if migration is needed
    const requiresMigration = !compatible && semver.lt(indexVersion, currentVersion);
    
    // Generate migration path if needed
    let migrationPath: string[] | undefined;
    if (requiresMigration) {
      migrationPath = this.generateMigrationPath(indexVersion, currentVersion);
    }
    
    return {
      version: indexVersion,
      compatible,
      requiresMigration,
      migrationPath
    };
  }
  
  /**
   * Generates migration path between versions
   */
  private static generateMigrationPath(from: string, to: string): string[] {
    const path: string[] = [];
    
    // Define migration steps
    if (semver.lt(from, '2.0.0')) {
      path.push('1.x-to-2.0.0');
    }
    if (semver.lt(from, '2.1.0') && semver.gte(to, '2.1.0')) {
      path.push('2.0.0-to-2.1.0');
    }
    
    return path;
  }
  
  /**
   * Checks if a reindex is recommended based on schema changes
   */
  static shouldRecommendReindex(indexVersion: string): {
    recommend: boolean;
    reason: string;
    severity: 'low' | 'medium' | 'high';
  } {
    const schemaInfo = this.validateSchema(indexVersion);
    
    if (!schemaInfo.compatible) {
      return {
        recommend: true,
        reason: `Schema version ${indexVersion} is incompatible with current version ${CORTEX_SCHEMA_VERSION}. A reindex is required to update data structures.`,
        severity: 'high'
      };
    }
    
    if (schemaInfo.requiresMigration) {
      return {
        recommend: true,
        reason: `Schema migration needed from ${indexVersion} to ${CORTEX_SCHEMA_VERSION}. Reindex recommended for optimal performance.`,
        severity: 'medium'
      };
    }
    
    return {
      recommend: false,
      reason: `Schema version ${indexVersion} is compatible with current version ${CORTEX_SCHEMA_VERSION}.`,
      severity: 'low'
    };
  }
  
  /**
   * Validates the structure of a persisted index
   */
  static validateIndexStructure(indexData: any): {
    valid: boolean;
    errors: string[];
    warnings: string[];
  } {
    const errors: string[] = [];
    const warnings: string[] = [];
    
    // Check required fields
    if (!indexData.version) {
      errors.push('Missing version field');
    }
    
    if (!indexData.timestamp) {
      errors.push('Missing timestamp field');
    }
    
    if (!Array.isArray(indexData.chunks)) {
      errors.push('Chunks field must be an array');
    } else {
      // Validate chunk structure
      for (let i = 0; i < Math.min(indexData.chunks.length, 10); i++) {
        const chunk = indexData.chunks[i];
        const chunkErrors = this.validateChunkStructure(chunk, i);
        errors.push(...chunkErrors);
      }
    }
    
    if (!indexData.metadata) {
      warnings.push('Missing metadata field');
    } else {
      if (!indexData.metadata.embeddingModel) {
        warnings.push('Missing embedding model information');
      }
    }
    
    return {
      valid: errors.length === 0,
      errors,
      warnings
    };
  }
  
  /**
   * Validates individual chunk structure
   */
  private static validateChunkStructure(chunk: any, index: number): string[] {
    const errors: string[] = [];
    
    const requiredFields = ['chunk_id', 'file_path', 'content', 'embedding'];
    for (const field of requiredFields) {
      if (!(field in chunk)) {
        errors.push(`Chunk ${index}: Missing required field '${field}'`);
      }
    }
    
    // Validate embedding
    if (chunk.embedding) {
      if (!Array.isArray(chunk.embedding)) {
        errors.push(`Chunk ${index}: Embedding must be an array`);
      } else if (chunk.embedding.length !== 384) {
        errors.push(`Chunk ${index}: Embedding must have 384 dimensions, got ${chunk.embedding.length}`);
      }
    }
    
    return errors;
  }
}