import { PersistentVectorStore } from './persistent-vector-store';
import { PersistentRelationshipStore } from './persistent-relationship-store';
import { logger } from './logging-utils';
import crypto from 'crypto';
import fs from 'fs/promises';
import path from 'path';

export interface RedundancyReport {
  timestamp: Date;
  type: 'vector' | 'relationship' | 'process' | 'code';
  duplicates: Array<{
    id: string;
    hash: string;
    count: number;
    firstSeen: Date;
    lastSeen: Date;
  }>;
  totalItems: number;
  duplicateCount: number;
  spaceSaved: number; // in bytes
}

export class RedundancyChecker {
  private vectorStore: PersistentVectorStore;
  private relationshipStore: PersistentRelationshipStore;

  constructor(vectorStore: PersistentVectorStore, relationshipStore: PersistentRelationshipStore) {
    this.vectorStore = vectorStore;
    this.relationshipStore = relationshipStore;
  }

  async checkVectorRedundancy(): Promise<RedundancyReport> {
    logger.info('🔍 Starting vector redundancy check...');
    
    const vectors = await this.vectorStore.listAllVectors();
    const hashMap = new Map<string, Array<{id: string, timestamp: Date}>>();
    
    for (const vector of vectors) {
      const hash = this.hashVector(vector.embedding);
      const key = hash;
      
      if (!hashMap.has(key)) {
        hashMap.set(key, []);
      }
      
      hashMap.get(key)!.push({
        id: vector.id,
        timestamp: new Date(vector.timestamp || Date.now())
      });
    }

    const duplicates = Array.from(hashMap.entries())
      .filter(([_, items]) => items.length > 1)
      .map(([hash, items]) => ({
        id: items[0].id,
        hash,
        count: items.length,
        firstSeen: new Date(Math.min(...items.map(i => i.timestamp.getTime()))),
        lastSeen: new Date(Math.max(...items.map(i => i.timestamp.getTime())))
      }));

    const totalItems = vectors.length;
    const duplicateCount = duplicates.reduce((sum, d) => sum + (d.count - 1), 0);
    const spaceSaved = duplicateCount * (vectors[0]?.embedding?.length || 1536) * 4; // float32 bytes

    return {
      timestamp: new Date(),
      type: 'vector',
      duplicates,
      totalItems,
      duplicateCount,
      spaceSaved
    };
  }

  async checkRelationshipRedundancy(): Promise<RedundancyReport> {
    logger.info('🔍 Starting relationship redundancy check...');
    
    const relationships = await this.relationshipStore.listAllRelationships();
    const hashMap = new Map<string, Array<{id: string, timestamp: Date}>>();
    
    for (const rel of relationships) {
      const hash = this.hashRelationship(rel);
      const key = hash;
      
      if (!hashMap.has(key)) {
        hashMap.set(key, []);
      }
      
      hashMap.get(key)!.push({
        id: rel.id,
        timestamp: new Date(rel.timestamp || Date.now())
      });
    }

    const duplicates = Array.from(hashMap.entries())
      .filter(([_, items]) => items.length > 1)
      .map(([hash, items]) => ({
        id: items[0].id,
        hash,
        count: items.length,
        firstSeen: new Date(Math.min(...items.map(i => i.timestamp.getTime()))),
        lastSeen: new Date(Math.max(...items.map(i => i.timestamp.getTime())))
      }));

    const totalItems = relationships.length;
    const duplicateCount = duplicates.reduce((sum, d) => sum + (d.count - 1), 0);
    
    // Estimate space saved (rough calculation)
    const spacePerRel = 1000; // rough estimate in bytes
    const spaceSaved = duplicateCount * spacePerRel;

    return {
      timestamp: new Date(),
      type: 'relationship',
      duplicates,
      totalItems,
      duplicateCount,
      spaceSaved
    };
  }

  async checkProcessRedundancy(): Promise<RedundancyReport> {
    logger.info('🔍 Checking for redundant processes...');
    
    try {
      const { execSync } = require('child_process');
      const processes = execSync('ps aux | grep -E "(embedding|embedder|worker)" | grep -v grep', 
        { encoding: 'utf8' })
        .split('\n')
        .filter(line => line.trim());
      
      const processMap = new Map<string, number>();
      
      for (const proc of processes) {
        const cmd = proc.split(' ').slice(10).join(' ');
        const key = cmd.trim();
        processMap.set(key, (processMap.get(key) || 0) + 1);
      }
      
      const duplicates = Array.from(processMap.entries())
        .filter(([_, count]) => count > 1)
        .map(([cmd, count]) => ({
          id: cmd.substring(0, 50),
          hash: this.hashText(cmd),
          count,
          firstSeen: new Date(),
          lastSeen: new Date()
        }));

      return {
        timestamp: new Date(),
        type: 'process',
        duplicates,
        totalItems: processes.length,
        duplicateCount: duplicates.reduce((sum, d) => sum + (d.count - 1), 0),
        spaceSaved: duplicates.reduce((sum, d) => sum + (d.count - 1) * 50 * 1024 * 1024, 0) // 50MB per process
      };
    } catch (error) {
      logger.warn('Process redundancy check failed:', error);
      return {
        timestamp: new Date(),
        type: 'process',
        duplicates: [],
        totalItems: 0,
        duplicateCount: 0,
        spaceSaved: 0
      };
    }
  }

  async checkCodeRedundancy(rootPath: string = '.'): Promise<RedundancyReport> {
    logger.info('🔍 Starting code redundancy check...');
    
    const files = await this.findSourceFiles(rootPath);
    const duplicateMap = new Map<string, Array<{file: string, line: number}>>();
    
    for (const file of files) {
      try {
        const content = await fs.readFile(file, 'utf8');
        const lines = content.split('\n');
        
        for (let i = 0; i < lines.length; i++) {
          const line = lines[i].trim();
          if (line.length > 50) { // Only check longer lines
            const hash = this.hashText(line);
            if (!duplicateMap.has(hash)) {
              duplicateMap.set(hash, []);
            }
            duplicateMap.get(hash)!.push({file, line: i + 1});
          }
        }
      } catch (error) {
        logger.debug(`Skipping ${file}: ${error}`);
      }
    }

    const duplicates = Array.from(duplicateMap.entries())
      .filter(([_, items]) => items.length > 2) // Only show duplicates appearing 3+ times
      .map(([hash, items]) => ({
        id: items[0].file,
        hash,
        count: items.length,
        firstSeen: new Date(),
        lastSeen: new Date()
      }));

    const totalItems = duplicates.reduce((sum, d) => sum + d.count, 0);
    const spacePerLine = 100; // estimate
    const spaceSaved = duplicates.reduce((sum, d) => sum + Math.max(0, d.count - 1) * spacePerLine, 0);

    return {
      timestamp: new Date(),
      type: 'code',
      duplicates,
      totalItems,
      duplicateCount: duplicates.reduce((sum, d) => sum + (d.count - 1), 0),
      spaceSaved
    };
  }

  private hashVector(vector: number[]): string {
    return crypto.createHash('sha256')
      .update(Buffer.from(new Float32Array(vector).buffer))
      .digest('hex')
      .substring(0, 16);
  }

  private hashRelationship(rel: any): string {
    const normalizedRel = {
      from: rel.from,
      to: rel.to,
      type: rel.type,
      metadata: this.sortKeys(rel.metadata || {})
    };
    return this.hashText(JSON.stringify(normalizedRel));
  }

  private hashText(text: string): string {
    return crypto.createHash('sha256')
      .update(text)
      .digest('hex')
      .substring(0, 16);
  }

  private sortKeys(obj: any): any {
    if (typeof obj !== 'object' || obj === null) return obj;
    if (Array.isArray(obj)) return obj.map(this.sortKeys.bind(this));
    
    const sorted = {};
    Object.keys(obj).sort().forEach(key => {
      sorted[key] = this.sortKeys(obj[key]);
    });
    return sorted;
  }

  private async findSourceFiles(rootPath: string): Promise<string[]> {
    const patterns = ['**/*.ts', '**/*.js', '!node_modules/**', '!dist/**', '!.git/**'];
    const globby = (await import('globby')).globby;
    return await globby(patterns, { cwd: rootPath });
  }

  async cleanupRedundancy(report: RedundancyReport): Promise<number> {
    logger.info(`🧹 Cleaning up ${report.type} redundancy...`);
    
    let cleaned = 0;
    
    if (report.type === 'vector') {
      for (const duplicate of report.duplicates) {
        const items = (await this.vectorStore.listAllVectors())
          .filter(v => this.hashVector(v.embedding) === duplicate.hash);
        
        // Keep the first one, remove the rest
        for (let i = 1; i < items.length; i++) {
          await this.vectorStore.deleteVector(items[i].id);
          cleaned++;
        }
      }
    } else if (report.type === 'relationship') {
      for (const duplicate of report.duplicates) {
        const rels = (await this.relationshipStore.listAllRelationships())
          .filter(r => this.hashRelationship(r) === duplicate.hash);
        
        // Keep the first one, remove the rest
        for (let i = 1; i < rels.length; i++) {
          await this.relationshipStore.deleteRelationship(rels[i].id);
          cleaned++;
        }
      }
    }

    logger.info(`✅ Cleaned ${cleaned} redundant ${report.type} items`);
    return cleaned;
  }

  async generateReport(): Promise<string> {
    const reports = await Promise.all([
      this.checkVectorRedundancy(),
      this.checkRelationshipRedundancy(),
      this.checkProcessRedundancy(),
      this.checkCodeRedundancy('./src')
    ]);

    let output = '# 🔍 Redundancy Report\n\n';
    output += `Generated: ${new Date().toISOString()}\n\n`;

    for (const report of reports) {
      output += `## ${report.type.toUpperCase()} Redundancy\n`;
      output += `- **Total Items**: ${report.totalItems}\n`;
      output += `- **Duplicates**: ${report.duplicateCount}\n`;
      output += `- **Space Wasted**: ${this.formatBytes(report.spaceSaved)}\n\n`;

      if (report.duplicates.length > 0) {
        output += '| Type | ID | Count | First Seen | Last Seen |\n';
        output += '|------|------|-------|------------|-----------|\n';
        
        for (const dup of report.duplicates.slice(0, 10)) {
          output += `| ${report.type} | ${dup.id} | ${dup.count} | ${dup.firstSeen.toISOString().substring(0, 10)} | ${dup.lastSeen.toISOString().substring(0, 10)} |\n`;
        }
        
        if (report.duplicates.length > 10) {
          output += `... and ${report.duplicates.length - 10} more\n`;
        }
      } else {
        output += '✅ No redundancy found\n';
      }
      
      output += '\n---\n\n';
    }

    return output;
  }

  private formatBytes(bytes: number): string {
    if (bytes === 0) return '0 B';
    const k = 1024;
    const sizes = ['B', 'KB', 'MB', 'GB'];
    const i = Math.floor(Math.log(bytes) / Math.log(k));
    return parseFloat((bytes / Math.pow(k, i)).toFixed(2)) + ' ' + sizes[i];
  }

  async schedulePeriodicChecks(): Promise<void> {
    const interval = 24 * 60 * 60 * 1000; // 24 hours
    
    const check = async () => {
      try {
        logger.info('🔍 Running scheduled redundancy check...');
        const report = await this.generateReport();
        
        await fs.mkdir('./reports', { recursive: true });
        await fs.writeFile(`./reports/redundancy-${Date.now()}.md`, report);
        
        logger.info('✅ Redundancy check completed');
      } catch (error) {
        logger.error('❌ Redundancy check failed:', error);
      }
    };

    // Run immediately
    await check();
    
    // Schedule regular checks
    setInterval(check, interval);
  }
}