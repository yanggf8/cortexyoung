import { COMPATIBLE_SCHEMA_VERSIONS, SchemaInfo } from './types';

export class SchemaValidator {
  
  /**
   * Simple compatibility check: Program 2.1 is compatible with Schema 1.x
   */
  static validateSchema(schemaVersion: string): SchemaInfo {
    const compatible = COMPATIBLE_SCHEMA_VERSIONS.includes(schemaVersion);
    
    return {
      version: schemaVersion,
      compatible,
      requiresMigration: false, // No migration needed for compatible schemas
      migrationPath: undefined
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