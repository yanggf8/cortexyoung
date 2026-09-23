/**
 * Integrated Pattern Detection Engine
 * 
 * Combines AST detection, confidence scoring, and conflict resolution
 * for comprehensive Express + Passport pattern analysis.
 */

import { ASTPassportDetector, PassportPattern } from './ast-detector-poc.js';
import { PassportConfidenceScorer, EvidenceItem, ConfidenceResult } from './confidence-scoring.js';
import { PatternConflictResolver, ConflictResolutionResult } from './pattern-conflict-resolver.js';

export interface IntegratedDetectionResult {
    patterns: {
        initialize: PatternResult;
        authenticate: PatternResult;
        strategy: PatternResult;
    };
    overallConfidence: number;
    conflictsResolved: number;
    processingTime: number;
    summary: string;
}

export interface PatternResult {
    detected: boolean;
    confidence: ConfidenceResult;
    conflicts: ConflictResolutionResult;
    evidence: EvidenceItem[];
    recommendation: string;
}

export class IntegratedPatternDetector {
    private astDetector: ASTPassportDetector;
    private confidenceScorer: PassportConfidenceScorer;
    private conflictResolver: PatternConflictResolver;

    constructor() {
        this.astDetector = new ASTPassportDetector();
        this.confidenceScorer = new PassportConfidenceScorer();
        this.conflictResolver = new PatternConflictResolver();
    }

    /**
     * Comprehensive pattern detection with conflict resolution
     */
    public async detectPatterns(projectPath: string): Promise<IntegratedDetectionResult> {
        const startTime = Date.now();
        
        // Step 1: AST-based pattern detection across project files
        const astResults = await this.scanProjectForPatterns(projectPath);
        
        // Step 2: Process each pattern type through the full pipeline
        const patterns = {
            initialize: await this.processPattern('initialize', astResults, projectPath),
            authenticate: await this.processPattern('authenticate', astResults, projectPath),
            strategy: await this.processPattern('strategy', astResults, projectPath),
        };

        // Step 3: Calculate overall metrics
        const overallConfidence = this.calculateOverallConfidence(patterns);
        const conflictsResolved = this.countResolvedConflicts(patterns);
        const processingTime = Date.now() - startTime;
        const summary = this.generateSummary(patterns, conflictsResolved);

        return {
            patterns,
            overallConfidence,
            conflictsResolved,
            processingTime,
            summary
        };
    }

    /**
     * Scan project files for Passport patterns using AST detector
     */
    private async scanProjectForPatterns(projectPath: string): Promise<PassportPattern[]> {
        const allPatterns: PassportPattern[] = [];
        const fs = await import('fs');
        const path = await import('path');
        const glob = await import('glob');
        
        // Find all JavaScript/TypeScript files in project
        const files = glob.sync('**/*.{js,ts}', { 
            cwd: projectPath,
            ignore: ['node_modules/**', 'dist/**', 'build/**', '*.d.ts']
        });
        
        for (const file of files) {
            const fullPath = path.join(projectPath, file);
            try {
                const patterns = await this.astDetector.detectPatternsInFile(fullPath);
                allPatterns.push(...patterns);
            } catch (error) {
                // Skip files that can't be parsed
                continue;
            }
        }
        
        return allPatterns;
    }

    /**
     * Process a single pattern type through the full pipeline
     */
    private async processPattern(
        patternType: 'initialize' | 'authenticate' | 'strategy' | 'route-protection',
        astResults: PassportPattern[],
        projectPath: string
    ): Promise<PatternResult> {
        // Extract evidence for this pattern type
        const evidence = this.extractEvidenceForPattern(patternType, astResults);
        
        // Step 1: Resolve conflicts
        const conflicts = this.conflictResolver.resolveConflicts(patternType, evidence);
        
        // Step 2: Calculate confidence on resolved patterns
        const confidence = this.confidenceScorer.calculateConfidence(
            patternType,
            conflicts.resolvedPatterns
        );
        
        // Step 3: Generate recommendation
        const recommendation = this.generateRecommendation(patternType, confidence, conflicts);

        return {
            detected: evidence.length > 0,
            confidence,
            conflicts,
            evidence: conflicts.resolvedPatterns,
            recommendation
        };
    }

    /**
     * Extract evidence items for specific pattern type from AST results
     */
    private extractEvidenceForPattern(
        patternType: 'initialize' | 'authenticate' | 'strategy' | 'route-protection',
        astResults: PassportPattern[]
    ): EvidenceItem[] {
        const evidence: EvidenceItem[] = [];

        // Filter patterns by type and convert to evidence format
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

        // Special handling for route-protection (derive from authenticate patterns)
        if (patternType === 'route-protection' && evidence.length === 0) {
            const authenticatePatterns = astResults.filter(p => p.type === 'authenticate');
            
            for (const pattern of authenticatePatterns) {
                // Route protection inferred from authenticate usage
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

    /**
     * Generate actionable recommendation based on detection results
     */
    private generateRecommendation(
        patternType: 'initialize' | 'authenticate' | 'strategy' | 'route-protection',
        confidence: ConfidenceResult,
        conflicts: ConflictResolutionResult
    ): string {
        if (confidence.score === 0) {
            return `No ${patternType} patterns detected. Consider implementing Passport ${patternType}.`;
        }

        let recommendation = '';

        // Base recommendation based on confidence level
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

        // Add conflict information
        if (conflicts.hasConflicts) {
            recommendation += `Resolved ${conflicts.conflicts.length} pattern conflict(s). `;
        }

        // Add specific guidance based on pattern type and evidence
        switch (patternType) {
            case 'initialize':
                if (confidence.score >= 75) {
                    recommendation += 'Use passport.initialize() for authentication setup.';
                } else {
                    recommendation += 'Verify passport.initialize() is properly configured.';
                }
                break;

            case 'authenticate':
                if (confidence.score >= 75) {
                    const strategy = this.extractStrategyFromEvidence(conflicts.resolvedPatterns);
                    recommendation += `Use passport.authenticate('${strategy}') for route protection.`;
                } else {
                    recommendation += 'Review authentication middleware configuration.';
                }
                break;

            case 'strategy':
                if (confidence.score >= 75) {
                    const strategies = this.extractStrategiesFromEvidence(conflicts.resolvedPatterns);
                    recommendation += `Configured strategies: ${strategies.join(', ')}.`;
                } else {
                    recommendation += 'Verify strategy configuration and setup.';
                }
                break;

            case 'route-protection':
                if (confidence.score >= 75) {
                    recommendation += 'Use detected authentication middleware for route protection.';
                } else {
                    recommendation += 'Consider implementing consistent route protection patterns.';
                }
                break;
        }

        // Add uncertainty warnings
        if (confidence.uncertainty.length > 0 && confidence.score < 75) {
            recommendation += ' ⚠️ ' + confidence.uncertainty[0];
        }

        return recommendation;
    }

    /**
     * Extract strategy name from authenticate evidence
     */
    private extractStrategyFromEvidence(evidence: EvidenceItem[]): string {
        for (const item of evidence) {
            const strategyMatch = item.pattern.match(/passport\.authenticate\(['"]([^'"]+)['"]/);
            if (strategyMatch) {
                return strategyMatch[1];
            }
        }
        return 'local';
    }

    /**
     * Extract strategy types from strategy evidence
     */
    private extractStrategiesFromEvidence(evidence: EvidenceItem[]): string[] {
        const strategies: string[] = [];
        
        for (const item of evidence) {
            if (item.pattern.includes('LocalStrategy')) {
                strategies.push('local');
            } else if (item.pattern.includes('JWTStrategy')) {
                strategies.push('jwt');
            } else if (item.pattern.includes('GoogleStrategy')) {
                strategies.push('google');
            } else if (item.pattern.includes('FacebookStrategy')) {
                strategies.push('facebook');
            }
        }

        return [...new Set(strategies)]; // Remove duplicates
    }

    /**
     * Calculate overall confidence across all patterns
     */
    private calculateOverallConfidence(patterns: {
        initialize: PatternResult;
        authenticate: PatternResult;
        strategy: PatternResult;
    }): number {
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

    /**
     * Count total conflicts resolved across all patterns
     */
    private countResolvedConflicts(patterns: {
        initialize: PatternResult;
        authenticate: PatternResult;
        strategy: PatternResult;
    }): number {
        return [
            patterns.initialize.conflicts.conflicts.length,
            patterns.authenticate.conflicts.conflicts.length,
            patterns.strategy.conflicts.conflicts.length,
        ].reduce((sum, count) => sum + count, 0);
    }

    /**
     * Generate comprehensive summary
     */
    private generateSummary(
        patterns: {
            initialize: PatternResult;
            authenticate: PatternResult;
            strategy: PatternResult;
            },
        conflictsResolved: number
    ): string {
        const detectedCount = Object.values(patterns).filter(p => p.detected).length;
        const avgConfidence = Math.round(this.calculateOverallConfidence(patterns));

        let summary = `Detected ${detectedCount}/4 Passport patterns with ${avgConfidence}% average confidence. `;
        
        if (conflictsResolved > 0) {
            summary += `Resolved ${conflictsResolved} conflicts using most-recent-wins strategy. `;
        }

        // Add specific findings
        const findings: string[] = [];
        
        if (patterns.initialize.detected && patterns.initialize.confidence.score >= 75) {
            findings.push('✅ Passport initialization configured');
        }
        
        if (patterns.strategy.detected && patterns.strategy.confidence.score >= 75) {
            const strategies = this.extractStrategiesFromEvidence(patterns.strategy.evidence);
            findings.push(`✅ ${strategies.length} authentication strategy(ies)`);
        }
        
        if (patterns.authenticate.detected && patterns.authenticate.confidence.score >= 75) {
            findings.push('✅ Route authentication implemented');
        }

        if (findings.length > 0) {
            summary += findings.join(', ') + '. ';
        }

        // Add recommendations for missing patterns
        const missing: string[] = [];
        
        if (!patterns.initialize.detected || patterns.initialize.confidence.score < 60) {
            missing.push('passport.initialize()');
        }
        
        if (!patterns.strategy.detected || patterns.strategy.confidence.score < 60) {
            missing.push('authentication strategy');
        }

        if (missing.length > 0) {
            summary += `Consider implementing: ${missing.join(', ')}.`;
        }

        return summary;
    }
}

// Example usage and testing
export async function testIntegratedDetection(projectPath: string = 'test/sample-project') {
    const detector = new IntegratedPatternDetector();
    
    console.log('🚀 Testing Integrated Pattern Detection...\n');
    console.log(`Analyzing project: ${projectPath}\n`);
    
    const result = await detector.detectPatterns(projectPath);
    
    console.log('=== INTEGRATED DETECTION RESULTS ===');
    console.log(`Overall Confidence: ${Math.round(result.overallConfidence)}%`);
    console.log(`Conflicts Resolved: ${result.conflictsResolved}`);
    console.log(`Processing Time: ${result.processingTime}ms`);
    console.log(`Summary: ${result.summary}\n`);
    
    // Display detailed results for each pattern
    const patternTypes = ['initialize', 'authenticate', 'strategy'] as const;
    
    for (const patternType of patternTypes) {
        const pattern = result.patterns[patternType];
        
        console.log(`--- ${patternType.toUpperCase()} PATTERN ---`);
        console.log(`  Detected: ${pattern.detected ? '✅' : '❌'}`);
        console.log(`  Confidence: ${pattern.confidence.score}% (${pattern.confidence.level})`);
        console.log(`  Evidence: ${pattern.evidence.length} item(s)`);
        
        if (pattern.conflicts.hasConflicts) {
            console.log(`  Conflicts: ${pattern.conflicts.conflicts.length} resolved`);
        }
        
        console.log(`  Recommendation: ${pattern.recommendation}`);
        
        if (pattern.confidence.uncertainty.length > 0) {
            console.log(`  Uncertainty: ${pattern.confidence.uncertainty.join(', ')}`);
        }
        
        console.log('');
    }
    
    return result;
}