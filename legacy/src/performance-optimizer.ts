/**
 * Performance Optimization for Express Project Analysis
 * 
 * Optimizes AST parsing, file scanning, and pattern detection
 * for faster ClaudeCat analysis of large Express projects.
 */

import * as fs from 'fs';
import * as path from 'path';
import { glob } from 'glob';

export interface PerformanceMetrics {
    totalTime: number;
    filesScanTime: number;
    astParseTime: number;
    patternDetectionTime: number;
    conflictResolutionTime: number;
    confidenceCalculationTime: number;
    filesProcessed: number;
    patternsDetected: number;
    avgTimePerFile: number;
    cacheHits: number;
    cacheEnabled: boolean;
}

export interface OptimizationConfig {
    enableCaching: boolean;
    maxConcurrentFiles: number;
    fileFilterPatterns: string[];
    skipDirectories: string[];
    maxFileSize: number; // in KB
    enableProgressiveScanning: boolean;
    priorityFiles: string[]; // High-priority files to process first
}

export class PerformanceOptimizer {
    private config: OptimizationConfig;
    private astCache: Map<string, any> = new Map();
    private fileStatsCache: Map<string, fs.Stats> = new Map();
    private metrics: PerformanceMetrics;

    constructor(config: Partial<OptimizationConfig> = {}) {
        this.config = {
            enableCaching: true,
            maxConcurrentFiles: 10,
            fileFilterPatterns: ['**/*.{js,ts,jsx,tsx}'],
            skipDirectories: ['node_modules', 'dist', 'build', '.git', 'coverage', '.nyc_output'],
            maxFileSize: 1024, // 1MB limit
            enableProgressiveScanning: true,
            priorityFiles: ['app.js', 'app.ts', 'server.js', 'server.ts', 'index.js', 'index.ts', 'main.js', 'main.ts'],
            ...config
        };

        this.metrics = this.initializeMetrics();
    }

    /**
     * Optimized file scanning with filtering and prioritization
     */
    public async scanProjectFiles(projectPath: string): Promise<string[]> {
        const startTime = Date.now();

        try {
            // Fast directory existence check
            if (!fs.existsSync(projectPath)) {
                throw new Error(`Project path does not exist: ${projectPath}`);
            }

            // Use optimized glob patterns
            const globPatterns = this.config.fileFilterPatterns;
            const ignorePatterns = this.config.skipDirectories.map(dir => `**/${dir}/**`);

            console.log(`📁 Scanning ${projectPath} with optimized patterns...`);

            const allFiles: string[] = [];
            
            // Process each pattern to avoid memory issues with large projects
            for (const pattern of globPatterns) {
                const files = await glob(pattern, {
                    cwd: projectPath,
                    ignore: ignorePatterns,
                    absolute: true
                });
                allFiles.push(...files);
            }

            // Remove duplicates and filter by file size
            const uniqueFiles = [...new Set(allFiles)];
            const filteredFiles = await this.filterFilesBySize(uniqueFiles);

            // Prioritize important files
            const prioritizedFiles = this.prioritizeFiles(filteredFiles, projectPath);

            this.metrics.filesScanTime = Date.now() - startTime;
            this.metrics.filesProcessed = prioritizedFiles.length;

            console.log(`  ✅ Found ${prioritizedFiles.length} files (${this.metrics.filesScanTime}ms)`);
            
            return prioritizedFiles;

        } catch (error) {
            console.error(`❌ File scanning failed: ${error}`);
            throw error;
        }
    }

    /**
     * Filter files by size to avoid processing extremely large files
     */
    private async filterFilesBySize(files: string[]): Promise<string[]> {
        const maxSizeBytes = this.config.maxFileSize * 1024;
        const filteredFiles: string[] = [];

        for (const file of files) {
            try {
                const stats = this.getFileCachedStats(file);
                if (stats && stats.size <= maxSizeBytes) {
                    filteredFiles.push(file);
                } else if (stats) {
                    console.log(`  ⚠️ Skipping large file: ${path.basename(file)} (${Math.round(stats.size / 1024)}KB)`);
                }
            } catch (error) {
                // Skip files that can't be accessed
                continue;
            }
        }

        return filteredFiles;
    }

    /**
     * Get file stats with caching
     */
    private getFileCachedStats(filePath: string): fs.Stats | null {
        if (this.config.enableCaching && this.fileStatsCache.has(filePath)) {
            this.metrics.cacheHits++;
            return this.fileStatsCache.get(filePath) || null;
        }

        try {
            const stats = fs.statSync(filePath);
            
            if (this.config.enableCaching) {
                this.fileStatsCache.set(filePath, stats);
            }
            
            return stats;
        } catch (error) {
            return null;
        }
    }

    /**
     * Prioritize important files for faster initial results
     */
    private prioritizeFiles(files: string[], projectPath: string): string[] {
        const priorityFiles: string[] = [];
        const regularFiles: string[] = [];

        for (const file of files) {
            const basename = path.basename(file).toLowerCase();
            const isPriority = this.config.priorityFiles.some(priority => 
                basename === priority.toLowerCase()
            );

            if (isPriority) {
                priorityFiles.push(file);
            } else {
                regularFiles.push(file);
            }
        }

        // Return priority files first, then regular files
        return [...priorityFiles, ...regularFiles];
    }

    /**
     * Batch process files with concurrency control
     */
    public async processFilesBatch<T>(
        files: string[],
        processor: (file: string) => Promise<T>,
        batchSize?: number
    ): Promise<T[]> {
        const actualBatchSize = batchSize || this.config.maxConcurrentFiles;
        const results: T[] = [];

        console.log(`🔄 Processing ${files.length} files (batch size: ${actualBatchSize})...`);

        // Process files in batches to control memory usage
        for (let i = 0; i < files.length; i += actualBatchSize) {
            const batch = files.slice(i, i + actualBatchSize);
            
            console.log(`  📦 Processing batch ${Math.floor(i / actualBatchSize) + 1}/${Math.ceil(files.length / actualBatchSize)} (${batch.length} files)`);

            // Process batch concurrently
            const batchPromises = batch.map(async file => {
                try {
                    return await processor(file);
                } catch (error) {
                    console.log(`    ⚠️ Failed to process ${path.basename(file)}: ${error}`);
                    return null;
                }
            });

            const batchResults = await Promise.all(batchPromises);
            
            // Filter out null results (failed processing)
            results.push(...batchResults.filter(result => result !== null) as T[]);

            // Small delay between batches to prevent overwhelming the system
            if (i + actualBatchSize < files.length) {
                await new Promise(resolve => setTimeout(resolve, 10));
            }
        }

        console.log(`  ✅ Processed ${results.length}/${files.length} files successfully`);
        
        return results;
    }

    /**
     * Progressive scanning - return results as they become available
     */
    public async *processFilesProgressively<T>(
        files: string[],
        processor: (file: string) => Promise<T>
    ): AsyncGenerator<T, void, unknown> {
        const batchSize = this.config.maxConcurrentFiles;

        for (let i = 0; i < files.length; i += batchSize) {
            const batch = files.slice(i, i + batchSize);
            
            const batchPromises = batch.map(async (file, index) => {
                try {
                    const result = await processor(file);
                    return { result, index: i + index };
                } catch (error) {
                    return { result: null, index: i + index };
                }
            });

            // Yield results as they complete
            for await (const { result } of batchPromises) {
                if (result !== null) {
                    yield result as T;
                }
            }
        }
    }

    /**
     * Smart caching based on file modification time
     */
    public getCachedResult<T>(filePath: string, key: string): T | null {
        if (!this.config.enableCaching) return null;

        const cacheKey = `${filePath}:${key}`;
        
        if (this.astCache.has(cacheKey)) {
            const cached = this.astCache.get(cacheKey);
            const stats = this.getFileCachedStats(filePath);
            
            // Check if file has been modified since caching
            if (stats && cached.timestamp >= stats.mtime.getTime()) {
                this.metrics.cacheHits++;
                return cached.data;
            } else {
                // Remove stale cache entry
                this.astCache.delete(cacheKey);
            }
        }

        return null;
    }

    /**
     * Cache result with file timestamp
     */
    public setCachedResult<T>(filePath: string, key: string, data: T): void {
        if (!this.config.enableCaching) return;

        const stats = this.getFileCachedStats(filePath);
        if (!stats) return;

        const cacheKey = `${filePath}:${key}`;
        this.astCache.set(cacheKey, {
            data,
            timestamp: stats.mtime.getTime()
        });
    }

    /**
     * Clear caches to free memory
     */
    public clearCaches(): void {
        this.astCache.clear();
        this.fileStatsCache.clear();
        console.log('🧹 Cleared performance caches');
    }

    /**
     * Get performance metrics
     */
    public getMetrics(): PerformanceMetrics {
        this.metrics.cacheEnabled = this.config.enableCaching;
        
        if (this.metrics.filesProcessed > 0) {
            this.metrics.avgTimePerFile = this.metrics.totalTime / this.metrics.filesProcessed;
        }

        return { ...this.metrics };
    }

    /**
     * Update timing metrics
     */
    public updateMetrics(phase: keyof PerformanceMetrics, time: number): void {
        if (typeof this.metrics[phase] === 'number') {
            (this.metrics as any)[phase] = time;
        }
    }

    /**
     * Initialize metrics
     */
    private initializeMetrics(): PerformanceMetrics {
        return {
            totalTime: 0,
            filesScanTime: 0,
            astParseTime: 0,
            patternDetectionTime: 0,
            conflictResolutionTime: 0,
            confidenceCalculationTime: 0,
            filesProcessed: 0,
            patternsDetected: 0,
            avgTimePerFile: 0,
            cacheHits: 0,
            cacheEnabled: false
        };
    }

    /**
     * Generate performance report
     */
    public generatePerformanceReport(): string {
        const metrics = this.getMetrics();
        
        let report = '# Performance Analysis Report\n\n';
        report += `**Total Processing Time**: ${metrics.totalTime}ms\n`;
        report += `**Files Processed**: ${metrics.filesProcessed}\n`;
        report += `**Patterns Detected**: ${metrics.patternsDetected}\n`;
        report += `**Average Time per File**: ${Math.round(metrics.avgTimePerFile)}ms\n`;
        report += `**Cache Enabled**: ${metrics.cacheEnabled ? 'Yes' : 'No'}\n`;
        
        if (metrics.cacheEnabled) {
            report += `**Cache Hits**: ${metrics.cacheHits}\n`;
        }
        
        report += '\n## Phase Breakdown:\n';
        report += `- **File Scanning**: ${metrics.filesScanTime}ms (${Math.round(metrics.filesScanTime / metrics.totalTime * 100)}%)\n`;
        report += `- **AST Parsing**: ${metrics.astParseTime}ms (${Math.round(metrics.astParseTime / metrics.totalTime * 100)}%)\n`;
        report += `- **Pattern Detection**: ${metrics.patternDetectionTime}ms (${Math.round(metrics.patternDetectionTime / metrics.totalTime * 100)}%)\n`;
        report += `- **Conflict Resolution**: ${metrics.conflictResolutionTime}ms (${Math.round(metrics.conflictResolutionTime / metrics.totalTime * 100)}%)\n`;
        report += `- **Confidence Calculation**: ${metrics.confidenceCalculationTime}ms (${Math.round(metrics.confidenceCalculationTime / metrics.totalTime * 100)}%)\n`;

        return report;
    }
}

// Example usage and testing
export async function testPerformanceOptimizations(projectPath: string = '.') {
    console.log('⚡ Testing Performance Optimizations...\n');

    const optimizer = new PerformanceOptimizer({
        enableCaching: true,
        maxConcurrentFiles: 8,
        maxFileSize: 512, // 512KB limit for testing
        enableProgressiveScanning: true
    });

    const startTime = Date.now();

    try {
        // Test optimized file scanning
        const files = await optimizer.scanProjectFiles(projectPath);
        console.log(`📊 Scanned ${files.length} files\n`);

        // Test batch processing
        const results = await optimizer.processFilesBatch(
            files.slice(0, 10), // Test with first 10 files
            async (file) => {
                // Simulate processing
                await new Promise(resolve => setTimeout(resolve, 10));
                return { file, size: fs.statSync(file).size };
            }
        );

        console.log(`📦 Batch processed ${results.length} files\n`);

        // Test progressive scanning
        console.log('🔄 Testing progressive scanning...');
        let progressiveCount = 0;
        for await (const result of optimizer.processFilesProgressively(
            files.slice(0, 5), // Test with first 5 files
            async (file) => {
                await new Promise(resolve => setTimeout(resolve, 5));
                return { file: path.basename(file) };
            }
        )) {
            progressiveCount++;
            console.log(`  ✅ Progressive result ${progressiveCount}: ${result.file}`);
        }

        const totalTime = Date.now() - startTime;
        optimizer.updateMetrics('totalTime', totalTime);
        optimizer.updateMetrics('filesProcessed', files.length);

        // Generate performance report
        const report = optimizer.generatePerformanceReport();
        console.log('\n' + report);

        return optimizer.getMetrics();

    } catch (error) {
        console.error('❌ Performance testing failed:', error);
        throw error;
    } finally {
        optimizer.clearCaches();
    }
}