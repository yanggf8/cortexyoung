/**
 * Optimized Pattern Detection Engine
 * 
 * High-performance version of the integrated pattern detector
 * with caching, batching, and progressive scanning optimizations.
 */

import { ASTPassportDetector } from './ast-detector-poc.js';
import { PassportConfidenceScorer } from './confidence-scoring.js';
import { PatternConflictResolver } from './pattern-conflict-resolver.js';
import { PerformanceOptimizer } from './performance-optimizer.js';
import { IntegratedDetectionResult, PatternResult } from './integrated-pattern-detector.js';

export class OptimizedPatternDetector {
    private astDetector: ASTPassportDetector;
    private confidenceScorer: PassportConfidenceScorer;
    private conflictResolver: PatternConflictResolver;
    private performanceOptimizer: PerformanceOptimizer;

    constructor() {
        this.astDetector = new ASTPassportDetector();
        this.confidenceScorer = new PassportConfidenceScorer();
        this.conflictResolver = new PatternConflictResolver();
        
        this.performanceOptimizer = new PerformanceOptimizer({
            enableCaching: true,
            maxConcurrentFiles: 8,
            maxFileSize: 1024, // 1MB limit
            enableProgressiveScanning: true,
            priorityFiles: [
                'app.js', 'app.ts', 'server.js', 'server.ts', 'index.js', 'index.ts',
                'main.js', 'main.ts', 'passport.js', 'passport.ts', 'auth.js', 'auth.ts'
            ]
        });
    }

    /**
     * High-performance pattern detection with optimization
     */
    public async detectPatterns(projectPath: string): Promise<IntegratedDetectionResult> {
        const startTime = Date.now();
        console.log(`🚀 Starting optimized pattern detection for ${projectPath}...\n`);

        try {
            // Phase 1: Optimized file scanning
            const scanStartTime = Date.now();
            const files = await this.performanceOptimizer.scanProjectFiles(projectPath);
            
            if (files.length === 0) {
                console.log('⚠️ No files found for analysis');
                return this.createEmptyResult(Date.now() - startTime);
            }

            this.performanceOptimizer.updateMetrics('filesScanTime', Date.now() - scanStartTime);

            // Phase 2: Batch AST parsing with caching
            const parseStartTime = Date.now();
            const allPatterns = await this.performBatchASTDetection(files);
            this.performanceOptimizer.updateMetrics('astParseTime', Date.now() - parseStartTime);

            console.log(`📊 Detected ${allPatterns.length} total patterns across ${files.length} files\n`);

            // Phase 3: Pattern processing pipeline
            const processingStartTime = Date.now();
            const patterns = await this.processPatternsPipeline(allPatterns, projectPath);
            this.performanceOptimizer.updateMetrics('patternDetectionTime', Date.now() - processingStartTime);

            // Phase 4: Calculate final metrics
            const totalTime = Date.now() - startTime;
            const overallConfidence = this.calculateOverallConfidence(patterns);
            const conflictsResolved = this.countResolvedConflicts(patterns);
            const summary = this.generateSummary(patterns, conflictsResolved);

            // Update performance metrics
            this.performanceOptimizer.updateMetrics('totalTime', totalTime);
            this.performanceOptimizer.updateMetrics('filesProcessed', files.length);
            this.performanceOptimizer.updateMetrics('patternsDetected', allPatterns.length);

            const result: IntegratedDetectionResult = {
                patterns,
                overallConfidence,
                conflictsResolved,
                processingTime: totalTime,
                summary
            };

            console.log(`✅ Optimized detection completed in ${totalTime}ms`);
            console.log(`📈 Performance: ${Math.round(files.length / (totalTime / 1000))} files/sec\n`);

            return result;

        } catch (error) {
            console.error('❌ Optimized pattern detection failed:', error);
            throw error;
        } finally {
            // Optional: Clear caches after processing to free memory
            // this.performanceOptimizer.clearCaches();
        }
    }

    /**
     * Batch AST detection with performance optimizations
     */
    private async performBatchASTDetection(files: string[]): Promise<any[]> {
        const allPatterns: any[] = [];

        const results = await this.performanceOptimizer.processFilesBatch(
            files,
            async (file) => {
                // Try to get cached result first
                const cacheKey = 'ast-patterns';
                const cached = this.performanceOptimizer.getCachedResult(file, cacheKey);
                
                if (cached) {
                    return cached;
                }

                // Parse file with AST detector
                try {
                    const patterns = await this.astDetector.detectPatternsInFile(file);
                    
                    // Cache the result
                    this.performanceOptimizer.setCachedResult(file, cacheKey, patterns);
                    
                    return patterns;
                } catch (error) {
                    // Skip files that can't be parsed
                    return [];
                }
            },
            6 // Slightly smaller batch size for AST parsing
        );

        // Flatten results
        for (const patterns of results) {
            if (Array.isArray(patterns)) {
                allPatterns.push(...patterns);
            }
        }

        return allPatterns;
    }

    /**
     * Progressive pattern detection - yields results as they become available
     */
    public async *detectPatternsProgressively(
        projectPath: string
    ): AsyncGenerator<{ patterns: any[], progress: number, total: number }, IntegratedDetectionResult, unknown> {
        console.log(`🔄 Starting progressive pattern detection for ${projectPath}...\n`);

        const startTime = Date.now();
        const files = await this.performanceOptimizer.scanProjectFiles(projectPath);
        
        if (files.length === 0) {
            return this.createEmptyResult(Date.now() - startTime);
        }

        const allPatterns: any[] = [];
        let processedFiles = 0;

        // Yield results progressively
        for await (const patterns of this.performanceOptimizer.processFilesProgressively(
            files,
            async (file) => {
                const cacheKey = 'ast-patterns';
                const cached = this.performanceOptimizer.getCachedResult(file, cacheKey);
                
                if (cached) {
                    return cached;
                }

                try {
                    const patterns = await this.astDetector.detectPatternsInFile(file);
                    this.performanceOptimizer.setCachedResult(file, cacheKey, patterns);
                    return patterns;
                } catch (error) {
                    return [];
                }
            }
        )) {
            if (Array.isArray(patterns)) {
                allPatterns.push(...patterns);
                processedFiles++;

                // Yield intermediate results
                yield {
                    patterns: [...allPatterns],
                    progress: processedFiles,
                    total: files.length
                };
            }
        }

        // Process final patterns pipeline
        const processedPatterns = await this.processPatternsPipeline(allPatterns, projectPath);
        
        const totalTime = Date.now() - startTime;
        const overallConfidence = this.calculateOverallConfidence(processedPatterns);
        const conflictsResolved = this.countResolvedConflicts(processedPatterns);
        const summary = this.generateSummary(processedPatterns, conflictsResolved);

        return {
            patterns: processedPatterns,
            overallConfidence,
            conflictsResolved,
            processingTime: totalTime,
            summary
        };
    }

    /**
     * Process patterns through the full pipeline (conflicts + confidence)
     */
    private async processPatternsPipeline(allPatterns: any[], projectPath: string): Promise<{
        initialize: PatternResult;
        authenticate: PatternResult;
        strategy: PatternResult;
    }> {
        const patternTypes = ['initialize', 'authenticate', 'strategy'] as const;
        const results: any = {};

        for (const patternType of patternTypes) {
            const conflictStartTime = Date.now();
            
            // Extract evidence for this pattern type
            const evidence = this.extractEvidenceForPattern(patternType, allPatterns);
            
            // Resolve conflicts
            const conflicts = this.conflictResolver.resolveConflicts(patternType, evidence);
            
            this.performanceOptimizer.updateMetrics('conflictResolutionTime', 
                Date.now() - conflictStartTime);

            // Calculate confidence
            const confidenceStartTime = Date.now();
            const confidence = this.confidenceScorer.calculateConfidence(
                patternType,
                conflicts.resolvedPatterns
            );
            
            this.performanceOptimizer.updateMetrics('confidenceCalculationTime', 
                Date.now() - confidenceStartTime);

            // Generate recommendation
            const recommendation = this.generateRecommendation(patternType, confidence, conflicts);

            const key = patternType;
            results[key] = {
                detected: evidence.length > 0,
                confidence,
                conflicts,
                evidence: conflicts.resolvedPatterns,
                recommendation
            };
        }

        return results;
    }

    /**
     * Get performance metrics from optimizer
     */
    public getPerformanceMetrics() {
        return this.performanceOptimizer.getMetrics();
    }

    /**
     * Generate performance report
     */
    public generatePerformanceReport(): string {
        return this.performanceOptimizer.generatePerformanceReport();
    }

    /**
     * Clear all caches to free memory
     */
    public clearCaches(): void {
        this.performanceOptimizer.clearCaches();
    }

    // Helper methods (copied from IntegratedPatternDetector)
    private extractEvidenceForPattern(patternType: string, astResults: any[]) {
        const evidence: any[] = [];
        const filteredPatterns = astResults.filter(pattern => pattern.type === patternType);
        
        for (const pattern of filteredPatterns) {
            evidence.push({
                file: pattern.file,
                line: pattern.line,
                pattern: pattern.pattern,
                matchQuality: pattern.confidence >= 90 ? 'exact' : 
                            pattern.confidence >= 70 ? 'partial' : 'inferred',
                context: pattern.evidence
            });
        }

        if (false && evidence.length === 0) { // Disabled route-protection logic
            const authenticatePatterns = astResults.filter(p => p.type === 'authenticate');
            
            for (const pattern of authenticatePatterns) {
                evidence.push({
                    file: pattern.file,
                    line: pattern.line,
                    pattern: pattern.pattern,
                    matchQuality: 'inferred',
                    context: pattern.evidence
                });
            }
        }

        return evidence;
    }

    private generateRecommendation(patternType: string, confidence: any, conflicts: any): string {
        if (confidence.score === 0) {
            return `No ${patternType} patterns detected. Consider implementing Passport ${patternType}.`;
        }

        let recommendation = '';
        
        switch (confidence.level) {
            case 'very-high':
                recommendation = `Strong ${patternType} implementation detected. `;
                break;
            case 'high':
                recommendation = `Good ${patternType} implementation detected. `;
                break;
            case 'medium':
                recommendation = `${patternType} implementation detected with some uncertainty. `;
                break;
            case 'low':
                recommendation = `Weak ${patternType} pattern detected. `;
                break;
        }

        if (conflicts.hasConflicts) {
            recommendation += `Resolved ${conflicts.conflicts.length} pattern conflict(s). `;
        }

        return recommendation;
    }

    private calculateOverallConfidence(patterns: any): number {
        const scores = [
            patterns.initialize.confidence.score,
            patterns.authenticate.confidence.score,
            patterns.strategy.confidence.score,
        ];

        const detectedPatterns = scores.filter(score => score > 0);
        
        if (detectedPatterns.length === 0) {
            return 0;
        }

        return detectedPatterns.reduce((sum, score) => sum + score, 0) / detectedPatterns.length;
    }

    private countResolvedConflicts(patterns: any): number {
        return [
            patterns.initialize.conflicts.conflicts.length,
            patterns.authenticate.conflicts.conflicts.length,
            patterns.strategy.conflicts.conflicts.length,
        ].reduce((sum, count) => sum + count, 0);
    }

    private generateSummary(patterns: any, conflictsResolved: number): string {
        const detectedCount = Object.values(patterns).filter((p: any) => p.detected).length;
        const avgConfidence = Math.round(this.calculateOverallConfidence(patterns));

        let summary = `Detected ${detectedCount}/4 Passport patterns with ${avgConfidence}% average confidence. `;
        
        if (conflictsResolved > 0) {
            summary += `Resolved ${conflictsResolved} conflicts using most-recent-wins strategy. `;
        }

        return summary;
    }

    private createEmptyResult(processingTime: number): IntegratedDetectionResult {
        const emptyPattern: PatternResult = {
            detected: false,
            confidence: {
                score: 0,
                level: 'low' as any,
                uncertainty: ['No evidence found for this pattern'],
                factors: {
                    evidenceCount: 0,
                    patternComplexity: 0,
                    contextQuality: 0,
                    consistency: 0,
                    recency: 0
                },
                evidence: [],
                reasoning: 'No patterns detected in project'
            },
            conflicts: {
                hasConflicts: false,
                conflicts: [],
                resolvedPatterns: [],
                resolutionStrategy: 'no-conflicts',
                confidence: 1.0
            },
            evidence: [],
            recommendation: 'No patterns detected in project'
        };

        return {
            patterns: {
                initialize: emptyPattern,
                authenticate: emptyPattern,
                strategy: emptyPattern,
            },
            overallConfidence: 0,
            conflictsResolved: 0,
            processingTime,
            summary: 'No patterns detected in project'
        };
    }
}

// Example usage and testing
export async function testOptimizedDetection(projectPath: string = '.') {
    console.log('⚡ Testing Optimized Pattern Detection...\n');

    const detector = new OptimizedPatternDetector();
    
    try {
        // Test standard optimized detection
        console.log('📊 Standard Optimized Detection:');
        const result1 = await detector.detectPatterns(projectPath);
        
        console.log('Performance Metrics:');
        const metrics = detector.getPerformanceMetrics();
        console.log(`  Files Processed: ${metrics.filesProcessed}`);
        console.log(`  Total Time: ${metrics.totalTime}ms`);
        console.log(`  Average Time per File: ${Math.round(metrics.avgTimePerFile)}ms`);
        console.log(`  Cache Hits: ${metrics.cacheHits}\n`);

        // Test progressive detection
        console.log('🔄 Progressive Detection:');
        let progressiveResult;
        
        for await (const progress of detector.detectPatternsProgressively(projectPath)) {
            if ('progress' in progress) {
                console.log(`  Progress: ${progress.progress}/${progress.total} files, ${progress.patterns.length} patterns found`);
            } else {
                progressiveResult = progress;
            }
        }

        // Generate performance report
        const report = detector.generatePerformanceReport();
        console.log('\n' + report);

        return { standard: result1, progressive: progressiveResult, metrics };

    } catch (error) {
        console.error('❌ Optimized detection test failed:', error);
        throw error;
    } finally {
        detector.clearCaches();
    }
}