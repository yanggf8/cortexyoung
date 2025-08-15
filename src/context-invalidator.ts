import { PersistentVectorStore } from './persistent-vector-store';
import { log } from './logging-utils';

interface ContextInvalidationEvent {
  filePath: string;
  changeType: 'structure' | 'content' | 'deleted';
  timestamp: number;
}

export class ContextInvalidator {
  private vectorStore: PersistentVectorStore;
  private invalidatedChunks = new Set<string>();

  constructor(vectorStore: PersistentVectorStore) {
    this.vectorStore = vectorStore;
    
    // Listen for semantic changes
    (process as any).on('cortex:contextInvalidated', (event: ContextInvalidationEvent) => {
      this.handleContextInvalidation(event);
    });
  }

  private async handleContextInvalidation(event: ContextInvalidationEvent): Promise<void> {
    log(`[ContextInvalidator] Invalidating context for ${event.filePath}`);
    
    if (event.changeType === 'deleted') {
      // Remove chunks for deleted files
      await this.vectorStore.removeChunksForFile(event.filePath);
    } else {
      // Mark chunks as stale for changed files
      const chunks = await this.vectorStore.getChunksForFile(event.filePath);
      chunks.forEach(chunk => this.invalidatedChunks.add(chunk.chunk_id));
    }
    
    // Trigger incremental reindexing if too many chunks are stale
    if (this.invalidatedChunks.size > 50) {
      log('[ContextInvalidator] Many chunks invalidated, triggering incremental reindex');
      (process as any).emit('cortex:triggerIncrementalReindex');
      this.invalidatedChunks.clear();
    }
  }

  getInvalidatedChunks(): string[] {
    return Array.from(this.invalidatedChunks);
  }

  clearInvalidatedChunks(): void {
    this.invalidatedChunks.clear();
  }

  getStats(): { invalidatedCount: number } {
    return { invalidatedCount: this.invalidatedChunks.size };
  }
}