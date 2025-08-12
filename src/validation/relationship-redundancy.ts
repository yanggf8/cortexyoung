import { PersistentRelationshipStore } from '../persistent-relationship-store';
import { logger } from '../logging-utils';
import crypto from 'crypto';

export interface RelationshipValidator {
  validateDuplicates(): Promise<{
    hasDuplicates: boolean;
    duplicates: Array<{
      hash: string;
      count: number;
      relationships: Array<{ id: string; from: string; to: string; type: string }>
    }>;
    stats: {
      total: number;
      unique: number
    }
  }>;
}

export class RedundantRelationshipValidator implements RelationshipValidator {
  private store: PersistentRelationshipStore;

  constructor(store: PersistentRelationshipStore) {
    this.store = store;
  }

  async validateDuplicates(): Promise<{
    hasDuplicates: boolean;
    duplicates: Array<{
      hash: string;
      count: number;
      relationships: Array<{ id: string; from: string; to: string; type: string }>
    }>;
    stats: {
      total: number;
      unique: number
    }
  }> {
    try {
      await this.store.initialize();
      
      const relationships = await this.store.listAllRelationships();
      const hashMap = new Map<string, Array<any>>();
      
      for (const rel of relationships) {
        const normalizedRel = this.normalizeRelationship(rel);
        const hash = this.generateHash(normalizedRel);
        
        if (!hashMap.has(hash)) {
          hashMap.set(hash, []);
        }
        
        hashMap.get(hash)!.push({
          id: rel.id,
          from: rel.from,
          to: rel.to,
          type: rel.type,
          metadata: rel.metadata || {}
        });
      }

      const duplicates = Array.from(hashMap.entries())
        .filter(([_, rels]) => rels.length > 1)
        .map(([hash, rels]) => ({
          hash,
          count: rels.length,
          relationships: rels
        }))
        .sort((a, b) => b.count - a.count);

      const uniqueHashes = new Set(hashMap.keys()).size;

      logger.info(`📊 Relationship Validation Results:`);
      logger.info(`- Total relationships: ${relationships.length}`);
      logger.info(`- Unique hashes: ${uniqueHashes}`);
      logger.info(`- Duplicate groups: ${duplicates.length}`);

      if (duplicates.length > 0) {
        logger.warn(`⚠️  Found ${duplicates.length} groups of duplicate relationships`);
        
        for (const duplicate of duplicates.slice(0, 5)) {
          logger.warn(`Duplicate group (${duplicate.count} instances):`);
          for (const rel of duplicate.relationships.slice(0, 3)) {
            logger.warn(`  - ${rel.type}: ${rel.from} → ${rel.to}`);
          }
          if (duplicate.relationships.length > 3) {
            logger.warn(`  ... and ${duplicate.relationships.length - 3} more`);
          }
        }
      }

      return {
        hasDuplicates: duplicates.length > 0,
        duplicates,
        stats: {
          total: relationships.length,
          unique: uniqueHashes
        }
      };
    } catch (error) {
      logger.error('❌ Relationship validation failed:', error);
      throw error;
    }
  }

  private normalizeRelationship(rel: any): any {
    return {
      from: rel.from || rel.source,
      to: rel.to || rel.target,
      type: rel.type,
      metadata: this.normalizeMetadata(rel.metadata),
      timestamp: Math.floor((rel.timestamp || new Date().getTime()) / 1000) // Round to seconds
    };
  }

  private normalizeMetadata(metadata: any): any {
    if (!metadata || typeof metadata !== 'object') return {};
    
    const normalized = {} as any;
    const keys = Object.keys(metadata).sort();
    
    for (const key of keys) {
      const value = metadata[key];
      
      if (typeof value === 'string' && value.trim().length < 100) {
        normalized[key] = value.trim();
      } else if (typeof value === 'number') {
        normalized[key] = value;
      } else if (typeof value === 'boolean') {
        normalized[key] = value;
      } else if (Array.isArray(value)) {
        normalized[key] = value.slice(0, 5).map(v => typeof v === 'string' ? v.trim() : v);
      }
    }
    
    return normalized;
  }

  private generateHash(obj: any): string {
    const normalized = JSON.stringify(obj, Object.keys(obj).sort());
    return crypto.createHash('md5').update(normalized).digest('hex');
  }

  async cleanupDuplicates(): Promise<{
    cleaned: number;
    savedSpace: number
  }> {
    logger.info('🧹 Starting duplicate relationship cleanup...');
    
    try {
      await this.store.initialize();
      
      const validation = await this.validateDuplicates();
      let cleaned = 0;
      let savedSpace = 0;
      
      for (const duplicate of validation.duplicates) {
        const relationships = duplicate.relationships;
        
        if (relationships.length < 2) continue;
        
        // Keep the first one, remove the rest
        const toKeep = relationships[0];
        const toRemove = relationships.slice(1);
        
        logger.info(`Removing ${toRemove.length} duplicate relationships...`);
        
        for (const rel of toRemove) {
          try {
            await this.store.deleteRelationship(rel.id);
            cleaned++;
            savedSpace += 1024; // Estimate 1KB per relationship
          } catch (error) {
            logger.error(`Failed to remove relationship ${rel.id}:`, error);
          }
        }
        
        logger.info(`Kept ${toKeep.id} (type: ${toKeep.type})`);
      }
      
      logger.info(`✅ Cleanup complete: removed ${cleaned} duplicate relationships`);
      
      return {
        cleaned,
        savedSpace
      };
    } catch (error) {
      logger.error('❌ Cleanup failed:', error);
      throw error;
    }
  }
}

async function validateRelationships() {
  const validator = new RedundantRelationshipValidator(
    new PersistentRelationshipStore()
  );
  
  console.log('🔍 Validating relationships for duplicates...');
  
  const result = await validator.validateDuplicates();
  
  console.log('\n📊 Summary:');
  console.log(`- Total relationships: ${result.stats.total}`);
  console.log(`- Unique relationships: ${result.stats.unique}`);
  console.log(`- Redundancy rate: ${((1 - result.stats.unique / result.stats.total) * 100).toFixed(2)}%`);
  
  if (result.hasDuplicates) {
    console.log('\n⚠️  Redundancy detected. Run cleanup?');
    console.log('Usage: npx ts-node -r ts-node/register src/validation/cleanup-duplicates.ts');
  } else {
    console.log('✅ No duplicates found!');
  }
}

export { RedundantRelationshipValidator };

if (require.main === module) {
  validateRelationships().catch(console.error);
}