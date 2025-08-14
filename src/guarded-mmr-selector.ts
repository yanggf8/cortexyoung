import { CodeChunk, QueryRequest, ContextPackage } from './types';
import { log, warn } from './logging-utils';
import { performance } from 'perf_hooks';

export interface MMRConfig {
  lambdaRelevance: number; // 0.7 = 70% relevance, 30% diversity
  maxTokenBudget: number;
  tokenCushionPercent: number; // 20% cushion
  diversityMetric: 'cosine' | 'jaccard' | 'semantic';
  minCriticalSetCoverage: number; // 0.95 = 95% coverage requirement
}

export interface CriticalSet {
  filePaths: string[];
  functionNames: string[];
  symbolNames: string[];
  confidence: number;
}

export interface MMRResult {
  selectedChunks: CodeChunk[];
  criticalSetCoverage: number;
  totalTokens: number;
  diversityScore: number;
  selectionTime: number;
  budgetUtilization: number;
}

export interface MMRSelectionStep {
  chunkId: string;
  relevanceScore: number;
  diversityScore: number;
  mmrScore: number;
  cumulativeTokens: number;
  isCritical: boolean;
}

export class GuardedMMRSelector {
  private config: MMRConfig;
  private tokenBudgetManager: TokenBudgetManager;
  private criticalSetExtractor: CriticalSetExtractor;

  constructor(config?: Partial<MMRConfig>) {
    this.config = {
      lambdaRelevance: 0.7,
      maxTokenBudget: 100000, // ~25k Claude tokens with 4:1 ratio
      tokenCushionPercent: 0.20,
      diversityMetric: 'cosine',
      minCriticalSetCoverage: 0.95,
      ...config
    };

    this.tokenBudgetManager = new TokenBudgetManager(this.config);
    this.criticalSetExtractor = new CriticalSetExtractor();
  }

  async selectOptimalChunks(
    candidates: CodeChunk[],
    query: QueryRequest,
    maxChunks?: number
  ): Promise<MMRResult> {
    const startTime = performance.now();
    
    // Security validation
    this.validateInputSecurity(candidates, query, maxChunks);
    
    log(`[MMR] Starting Guarded MMR selection candidates=${candidates.length} budget=${this.config.maxTokenBudget}`);

    // Phase 1: Extract critical set from query
    const criticalSet = await this.criticalSetExtractor.extractCriticalSet(query);
    log(`[MMR] Critical set extracted files=${criticalSet.filePaths.length} functions=${criticalSet.functionNames.length} confidence=${criticalSet.confidence.toFixed(2)}`);

    // Phase 2: Identify critical chunks (guaranteed inclusion)
    const criticalChunks = this.identifyCriticalChunks(candidates, criticalSet);
    const nonCriticalCandidates = candidates.filter(c => 
      !criticalChunks.some(cc => cc.chunk_id === c.chunk_id)
    );

    log(`[MMR] Critical chunks identified=${criticalChunks.length} remaining_candidates=${nonCriticalCandidates.length}`);

    // Phase 3: Validate critical set fits in budget
    const criticalTokens = this.tokenBudgetManager.calculateTokens(criticalChunks);
    const availableTokens = Math.floor(this.config.maxTokenBudget * (1 - this.config.tokenCushionPercent)) - criticalTokens;
    
    if (availableTokens <= 0) {
      warn(`[MMR] Critical set exceeds token budget critical=${criticalTokens} budget=${this.config.maxTokenBudget}`);
      // Emergency fallback - select most important critical chunks that fit
      const reducedCritical = this.emergencyReduction(criticalChunks, Math.floor(this.config.maxTokenBudget * 0.8));
      return {
        selectedChunks: reducedCritical,
        criticalSetCoverage: reducedCritical.length / criticalChunks.length,
        totalTokens: this.tokenBudgetManager.calculateTokens(reducedCritical),
        diversityScore: 0,
        selectionTime: performance.now() - startTime,
        budgetUtilization: 1.0
      };
    }

    // Phase 4: MMR selection from remaining candidates
    const selectedNonCritical = await this.runMMRSelection(
      nonCriticalCandidates,
      availableTokens,
      criticalChunks,
      maxChunks ? maxChunks - criticalChunks.length : undefined
    );

    // Phase 5: Combine and compute final metrics
    const allSelected = [...criticalChunks, ...selectedNonCritical];
    const totalTokens = this.tokenBudgetManager.calculateTokens(allSelected);
    const diversityScore = this.calculateDiversityScore(allSelected);
    const selectionTime = performance.now() - startTime;

    const result: MMRResult = {
      selectedChunks: allSelected,
      criticalSetCoverage: criticalChunks.length / Math.max(criticalSet.filePaths.length + criticalSet.functionNames.length, 1),
      totalTokens,
      diversityScore,
      selectionTime,
      budgetUtilization: totalTokens / this.config.maxTokenBudget
    };

    log(`[MMR] Selection complete chunks=${result.selectedChunks.length} tokens=${result.totalTokens} critical_coverage=${(result.criticalSetCoverage * 100).toFixed(1)}% time=${selectionTime.toFixed(2)}ms`);

    return result;
  }

  private identifyCriticalChunks(candidates: CodeChunk[], criticalSet: CriticalSet): CodeChunk[] {
    const critical: CodeChunk[] = [];

    for (const chunk of candidates) {
      let isCritical = false;

      // File path matching
      if (criticalSet.filePaths.some(path => 
        chunk.file_path.includes(path) || path.includes(chunk.file_path)
      )) {
        isCritical = true;
      }

      // Function name matching
      if (chunk.function_name && criticalSet.functionNames.some(name =>
        chunk.function_name?.toLowerCase().includes(name.toLowerCase()) ||
        name.toLowerCase().includes(chunk.function_name?.toLowerCase() || '')
      )) {
        isCritical = true;
      }

      // Symbol name matching
      if (chunk.symbol_name && criticalSet.symbolNames.some(name =>
        chunk.symbol_name?.toLowerCase().includes(name.toLowerCase()) ||
        name.toLowerCase().includes(chunk.symbol_name?.toLowerCase() || '')
      )) {
        isCritical = true;
      }

      if (isCritical) {
        critical.push(chunk);
      }
    }

    // Sort critical chunks by relevance score to prioritize most important
    return critical.sort((a, b) => 
      (b.relevance_score || b.similarity_score || 0) - (a.relevance_score || a.similarity_score || 0)
    );
  }

  private async runMMRSelection(
    candidates: CodeChunk[],
    availableTokens: number,
    selectedChunks: CodeChunk[],
    maxChunks?: number
  ): Promise<CodeChunk[]> {
    const result: CodeChunk[] = [];
    const remaining = [...candidates];
    let currentTokens = 0;

    // Sort candidates by relevance for initial ordering
    remaining.sort((a, b) => 
      (b.relevance_score || b.similarity_score || 0) - (a.relevance_score || a.similarity_score || 0)
    );

    while (remaining.length > 0 && currentTokens < availableTokens) {
      if (maxChunks && result.length >= maxChunks) break;

      let bestChunk: CodeChunk | null = null;
      let bestScore = -Infinity;
      let bestIndex = -1;

      // Find chunk with best MMR score
      for (let i = 0; i < remaining.length; i++) {
        const candidate = remaining[i];
        const candidateTokens = this.tokenBudgetManager.estimateChunkTokens(candidate);
        
        // Skip if would exceed token budget
        if (currentTokens + candidateTokens > availableTokens) continue;

        // Calculate MMR score
        const relevanceScore = candidate.relevance_score || candidate.similarity_score || 0;
        const diversityScore = this.calculateDiversityFromSelected(candidate, [...selectedChunks, ...result]);
        const mmrScore = this.config.lambdaRelevance * relevanceScore + 
                        (1 - this.config.lambdaRelevance) * diversityScore;

        if (mmrScore > bestScore) {
          bestScore = mmrScore;
          bestChunk = candidate;
          bestIndex = i;
        }
      }

      if (bestChunk) {
        result.push(bestChunk);
        remaining.splice(bestIndex, 1);
        currentTokens += this.tokenBudgetManager.estimateChunkTokens(bestChunk);
      } else {
        // No more chunks fit in budget
        break;
      }
    }

    return result;
  }

  private calculateDiversityFromSelected(candidate: CodeChunk, selected: CodeChunk[]): number {
    if (selected.length === 0) return 1.0;

    let minSimilarity = Infinity;

    for (const selectedChunk of selected) {
      const similarity = this.calculateSimilarity(candidate, selectedChunk);
      minSimilarity = Math.min(minSimilarity, similarity);
    }

    // Return diversity (1 - similarity)
    return 1 - Math.max(0, Math.min(1, minSimilarity));
  }

  private calculateSimilarity(chunk1: CodeChunk, chunk2: CodeChunk): number {
    switch (this.config.diversityMetric) {
      case 'cosine':
        return this.cosineSimilarity(chunk1.embedding, chunk2.embedding);
      
      case 'jaccard':
        return this.jaccardSimilarity(chunk1, chunk2);
      
      case 'semantic':
        return this.semanticSimilarity(chunk1, chunk2);
      
      default:
        return this.cosineSimilarity(chunk1.embedding, chunk2.embedding);
    }
  }

  private cosineSimilarity(a: number[], b: number[]): number {
    if (a.length !== b.length) return 0;

    let dotProduct = 0;
    let normA = 0;
    let normB = 0;

    for (let i = 0; i < a.length; i++) {
      dotProduct += a[i] * b[i];
      normA += a[i] * a[i];
      normB += b[i] * b[i];
    }

    if (normA === 0 || normB === 0) return 0;
    return dotProduct / (Math.sqrt(normA) * Math.sqrt(normB));
  }

  private jaccardSimilarity(chunk1: CodeChunk, chunk2: CodeChunk): number {
    // File similarity
    if (chunk1.file_path === chunk2.file_path) return 0.8;
    
    // Type similarity
    if (chunk1.chunk_type === chunk2.chunk_type) return 0.6;
    
    // Content word overlap
    const words1 = new Set(chunk1.content.toLowerCase().match(/\w+/g) || []);
    const words2 = new Set(chunk2.content.toLowerCase().match(/\w+/g) || []);
    
    const intersection = new Set([...words1].filter(x => words2.has(x)));
    const union = new Set([...words1, ...words2]);
    
    return union.size > 0 ? intersection.size / union.size : 0;
  }

  private semanticSimilarity(chunk1: CodeChunk, chunk2: CodeChunk): number {
    // Combine multiple similarity signals
    const cosineSim = this.cosineSimilarity(chunk1.embedding, chunk2.embedding);
    const jaccardSim = this.jaccardSimilarity(chunk1, chunk2);
    
    // Weighted combination
    return 0.7 * cosineSim + 0.3 * jaccardSim;
  }

  private calculateDiversityScore(chunks: CodeChunk[]): number {
    if (chunks.length <= 1) return 1.0;

    let totalSimilarity = 0;
    let pairCount = 0;

    for (let i = 0; i < chunks.length; i++) {
      for (let j = i + 1; j < chunks.length; j++) {
        totalSimilarity += this.calculateSimilarity(chunks[i], chunks[j]);
        pairCount++;
      }
    }

    const avgSimilarity = pairCount > 0 ? totalSimilarity / pairCount : 0;
    return 1 - avgSimilarity; // Diversity is inverse of average similarity
  }

  private emergencyReduction(chunks: CodeChunk[], maxTokens: number): CodeChunk[] {
    // Sort by relevance score and take highest scoring chunks that fit
    const sorted = chunks.sort((a, b) => 
      (b.relevance_score || b.similarity_score || 0) - (a.relevance_score || a.similarity_score || 0)
    );

    const result: CodeChunk[] = [];
    let currentTokens = 0;

    for (const chunk of sorted) {
      const chunkTokens = this.tokenBudgetManager.estimateChunkTokens(chunk);
      if (currentTokens + chunkTokens <= maxTokens) {
        result.push(chunk);
        currentTokens += chunkTokens;
      }
    }

    warn(`[MMR] Emergency reduction applied selected=${result.length}/${chunks.length} tokens=${currentTokens}`);
    return result;
  }

  private validateInputSecurity(candidates: CodeChunk[], query: QueryRequest, maxChunks?: number): void {
    // Validate candidates array
    if (!Array.isArray(candidates)) {
      throw new Error('MMR: candidates must be an array');
    }
    
    if (candidates.length > 10000) {
      throw new Error(`MMR: candidates array too large (${candidates.length} > 10000)`);
    }

    // Validate query object
    if (!query || typeof query !== 'object') {
      throw new Error('MMR: query must be a valid object');
    }

    if (!query.task || typeof query.task !== 'string') {
      throw new Error('MMR: query.task must be a non-empty string');
    }

    if (query.task.length > 10000) {
      throw new Error(`MMR: query.task too long (${query.task.length} > 10000 characters)`);
    }

    // Validate maxChunks
    if (maxChunks !== undefined) {
      if (typeof maxChunks !== 'number' || maxChunks < 1 || maxChunks > 1000) {
        throw new Error(`MMR: maxChunks must be between 1 and 1000 (got ${maxChunks})`);
      }
    }

    // Validate each candidate chunk
    for (let i = 0; i < Math.min(candidates.length, 100); i++) { // Sample validation
      const chunk = candidates[i];
      if (!chunk.chunk_id || !chunk.content || !chunk.embedding) {
        throw new Error(`MMR: invalid chunk structure at index ${i}`);
      }
      
      if (chunk.content.length > 50000) {
        throw new Error(`MMR: chunk content too large at index ${i} (${chunk.content.length} > 50000 characters)`);
      }
    }

    // Resource limits based on configuration
    const estimatedMemoryUsage = candidates.length * 1024; // Rough estimate
    if (estimatedMemoryUsage > 100 * 1024 * 1024) { // 100MB limit
      warn(`[MMR] Large memory usage estimated: ${(estimatedMemoryUsage / 1024 / 1024).toFixed(2)}MB`);
    }
  }
}

class TokenBudgetManager {
  private config: MMRConfig;
  private tokenEstimationCache = new Map<string, number>();

  constructor(config: MMRConfig) {
    this.config = config;
  }

  calculateTokens(chunks: CodeChunk[]): number {
    return chunks.reduce((total, chunk) => total + this.estimateChunkTokens(chunk), 0);
  }

  estimateChunkTokens(chunk: CodeChunk): number {
    const cacheKey = chunk.chunk_id;
    
    if (this.tokenEstimationCache.has(cacheKey)) {
      return this.tokenEstimationCache.get(cacheKey)!;
    }

    // Improved token estimation considering code structure
    const content = chunk.content;
    let tokens = 0;

    // Base content tokens (improved from 4-char rule)
    tokens += Math.ceil(content.length / 3.5); // Empirical: code averages ~3.5 chars per token

    // Add overhead for code structure tokens
    const lines = content.split('\n').length;
    const codeBlocks = (content.match(/```/g)?.length || 0) / 2;
    const functionDefs = (content.match(/function|=>|{/g)?.length || 0);
    
    tokens += lines * 0.1; // Line break tokens
    tokens += codeBlocks * 10; // Code block delimiters
    tokens += functionDefs * 2; // Function definition overhead
    
    // Metadata tokens
    tokens += 20; // File path, symbol names, etc.

    const finalTokens = Math.ceil(tokens);
    this.tokenEstimationCache.set(cacheKey, finalTokens);
    
    return finalTokens;
  }

  getRemainingBudget(usedTokens: number): number {
    const effectiveBudget = Math.floor(this.config.maxTokenBudget * (1 - this.config.tokenCushionPercent));
    return Math.max(0, effectiveBudget - usedTokens);
  }

  clearCache(): void {
    this.tokenEstimationCache.clear();
  }
}

class CriticalSetExtractor {
  private filePathPattern = /[\/\\]?([a-zA-Z0-9._-]+\.[a-zA-Z]+)/g;
  private functionPattern = /\b([a-zA-Z_][a-zA-Z0-9_]*)\s*\(/g;
  private symbolPattern = /\b([A-Z][a-zA-Z0-9]*|[a-z][a-zA-Z0-9]*(?:[A-Z][a-zA-Z0-9]*)+)\b/g;

  async extractCriticalSet(query: QueryRequest): Promise<CriticalSet> {
    const task = query.task.toLowerCase();
    
    // Extract file paths
    const filePaths = this.extractFilePaths(task);
    
    // Extract function names
    const functionNames = this.extractFunctionNames(task);
    
    // Extract symbol names (classes, interfaces, variables)
    const symbolNames = this.extractSymbolNames(task);
    
    // Calculate confidence based on extraction success
    const confidence = this.calculateExtractionConfidence(query.task, filePaths, functionNames, symbolNames);

    return {
      filePaths,
      functionNames,
      symbolNames,
      confidence
    };
  }

  private extractFilePaths(text: string): string[] {
    const paths: string[] = [];
    let match;

    while ((match = this.filePathPattern.exec(text)) !== null) {
      const path = match[1];
      if (this.isValidFilePath(path)) {
        paths.push(path);
      }
    }

    // Also look for explicit mentions
    const explicitPaths = text.match(/(?:file|path|in)\s+([a-zA-Z0-9._/-]+\.[a-zA-Z]+)/gi);
    if (explicitPaths) {
      explicitPaths.forEach(match => {
        const path = match.split(/\s+/).pop();
        if (path && this.isValidFilePath(path)) {
          paths.push(path);
        }
      });
    }

    return [...new Set(paths)]; // Remove duplicates
  }

  private extractFunctionNames(text: string): string[] {
    const names: string[] = [];
    
    // Look for function mentions
    const functionMentions = text.match(/(?:function|method|call(?:ing)?)\s+([a-zA-Z_][a-zA-Z0-9_]*)/gi);
    if (functionMentions) {
      functionMentions.forEach(match => {
        const name = match.split(/\s+/).pop();
        if (name && this.isValidIdentifier(name)) {
          names.push(name);
        }
      });
    }

    // Look for parentheses patterns (likely function calls)
    let match;
    while ((match = this.functionPattern.exec(text)) !== null) {
      const name = match[1];
      if (this.isValidIdentifier(name)) {
        names.push(name);
      }
    }

    return [...new Set(names)];
  }

  private extractSymbolNames(text: string): string[] {
    const names: string[] = [];
    
    // Look for class/interface mentions
    const classMentions = text.match(/(?:class|interface|type|component)\s+([A-Z][a-zA-Z0-9]*)/gi);
    if (classMentions) {
      classMentions.forEach(match => {
        const name = match.split(/\s+/).pop();
        if (name && this.isValidIdentifier(name)) {
          names.push(name);
        }
      });
    }

    // Look for camelCase/PascalCase identifiers
    let match;
    while ((match = this.symbolPattern.exec(text)) !== null) {
      const name = match[1];
      if (this.isValidIdentifier(name) && name.length > 2) {
        names.push(name);
      }
    }

    return [...new Set(names)];
  }

  private isValidFilePath(path: string): boolean {
    const validExtensions = ['.ts', '.js', '.tsx', '.jsx', '.py', '.java', '.cpp', '.c', '.h'];
    return validExtensions.some(ext => path.toLowerCase().endsWith(ext)) && 
           path.length > 3 && 
           !path.includes('node_modules');
  }

  private isValidIdentifier(name: string): boolean {
    return /^[a-zA-Z_][a-zA-Z0-9_]*$/.test(name) && 
           name.length >= 2 && 
           !['and', 'or', 'not', 'the', 'for', 'with', 'from'].includes(name.toLowerCase());
  }

  private calculateExtractionConfidence(
    originalQuery: string,
    filePaths: string[],
    functionNames: string[],
    symbolNames: string[]
  ): number {
    const totalMentions = filePaths.length + functionNames.length + symbolNames.length;
    const queryLength = originalQuery.length;
    
    if (totalMentions === 0) return 0.1; // Low confidence if nothing extracted
    
    // Higher confidence for more specific queries with clear identifiers
    let confidence = Math.min(0.9, 0.3 + (totalMentions * 0.15));
    
    // Boost confidence for explicit file mentions
    if (filePaths.length > 0) confidence += 0.2;
    
    // Boost confidence for function patterns
    if (functionNames.length > 0) confidence += 0.1;
    
    return Math.min(0.95, confidence);
  }
}