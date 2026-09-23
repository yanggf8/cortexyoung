/**
 * Automated Accuracy Measurement Framework for Express + Passport Projects
 * 
 * Validates pattern detection accuracy against ground truth dataset
 * and provides comprehensive accuracy metrics for ClaudeCat improvements.
 */

import { IntegratedPatternDetector, IntegratedDetectionResult } from './integrated-pattern-detector.js';
import * as fs from 'fs';
import * as path from 'path';

export interface GroundTruthPattern {
    patternType: 'initialize' | 'authenticate' | 'strategy' | 'route-protection';
    expectedPattern: string;
    file: string;
    line: number;
    confidence: 'high' | 'medium' | 'low';
    description: string;
}

export interface GroundTruthProject {
    name: string;
    path: string;
    description: string;
    expectedPatterns: GroundTruthPattern[];
    framework: 'express' | 'nestjs' | 'fastify';
    authLibrary: 'passport' | 'auth0' | 'custom';
}

export interface AccuracyMetrics {
    projectName: string;
    overallAccuracy: number;
    patternAccuracy: {
        initialize: PatternAccuracyResult;
        authenticate: PatternAccuracyResult;
        strategy: PatternAccuracyResult;
    };
    falsePositiveRate: number;
    falseNegativeRate: number;
    confidenceAccuracy: number;
    processingTime: number;
}

export interface PatternAccuracyResult {
    truePositives: number;
    falsePositives: number;
    falseNegatives: number;
    precision: number;
    recall: number;
    f1Score: number;
    detectedPatterns: string[];
    missedPatterns: string[];
    incorrectPatterns: string[];
}

export interface FrameworkAccuracyReport {
    framework: string;
    projectsAnalyzed: number;
    averageAccuracy: number;
    accuracyByPattern: Record<string, number>;
    commonFailures: string[];
    recommendations: string[];
    detailedResults: AccuracyMetrics[];
}

export class AccuracyMeasurementFramework {
    private detector: IntegratedPatternDetector;
    private groundTruthProjects: GroundTruthProject[];

    constructor() {
        this.detector = new IntegratedPatternDetector();
        this.groundTruthProjects = [];
    }

    /**
     * Load ground truth dataset from JSON file
     */
    public loadGroundTruthDataset(datasetPath: string): void {
        try {
            const datasetContent = fs.readFileSync(datasetPath, 'utf-8');
            const dataset = JSON.parse(datasetContent);
            this.groundTruthProjects = dataset.projects || [];
            
            console.log(`📊 Loaded ${this.groundTruthProjects.length} ground truth projects`);
        } catch (error) {
            console.error('❌ Failed to load ground truth dataset:', error);
            throw error;
        }
    }

    /**
     * Run accuracy measurement on all ground truth projects
     */
    public async measureAccuracy(): Promise<FrameworkAccuracyReport[]> {
        const reports: FrameworkAccuracyReport[] = [];
        const frameworks = [...new Set(this.groundTruthProjects.map(p => p.framework))];

        for (const framework of frameworks) {
            const frameworkProjects = this.groundTruthProjects.filter(p => p.framework === framework);
            const report = await this.measureFrameworkAccuracy(framework, frameworkProjects);
            reports.push(report);
        }

        return reports;
    }

    /**
     * Measure accuracy for a specific framework
     */
    private async measureFrameworkAccuracy(
        framework: string,
        projects: GroundTruthProject[]
    ): Promise<FrameworkAccuracyReport> {
        console.log(`\n🔍 Measuring accuracy for ${framework} (${projects.length} projects)...`);
        
        const results: AccuracyMetrics[] = [];
        const accuracyByPattern: Record<string, number[]> = {
            initialize: [],
            authenticate: [],
            strategy: [],
        };

        for (const project of projects) {
            if (!fs.existsSync(project.path)) {
                console.log(`⚠️ Skipping ${project.name} - path not found: ${project.path}`);
                continue;
            }

            console.log(`  📁 Analyzing ${project.name}...`);
            
            try {
                const metrics = await this.measureProjectAccuracy(project);
                results.push(metrics);

                // Collect pattern accuracy
                accuracyByPattern.initialize.push(metrics.patternAccuracy.initialize.f1Score);
                accuracyByPattern.authenticate.push(metrics.patternAccuracy.authenticate.f1Score);
                accuracyByPattern.strategy.push(metrics.patternAccuracy.strategy.f1Score);
            } catch (error) {
                console.log(`    ❌ Failed to analyze ${project.name}: ${error}`);
                continue;
            }
        }

        // Calculate averages
        const averageAccuracy = results.reduce((sum, r) => sum + r.overallAccuracy, 0) / results.length || 0;
        const avgAccuracyByPattern = Object.fromEntries(
            Object.entries(accuracyByPattern).map(([pattern, scores]) => [
                pattern,
                scores.reduce((sum, score) => sum + score, 0) / scores.length || 0
            ])
        );

        const commonFailures = this.identifyCommonFailures(results);
        const recommendations = this.generateRecommendations(avgAccuracyByPattern, commonFailures);

        return {
            framework,
            projectsAnalyzed: results.length,
            averageAccuracy,
            accuracyByPattern: avgAccuracyByPattern,
            commonFailures,
            recommendations,
            detailedResults: results
        };
    }

    /**
     * Measure accuracy for a single project
     */
    private async measureProjectAccuracy(project: GroundTruthProject): Promise<AccuracyMetrics> {
        const startTime = Date.now();
        
        // Run pattern detection
        const detectionResult = await this.detector.detectPatterns(project.path);
        const processingTime = Date.now() - startTime;

        // Compare with ground truth
        const patternAccuracy = {
            initialize: this.calculatePatternAccuracy('initialize', project, detectionResult),
            authenticate: this.calculatePatternAccuracy('authenticate', project, detectionResult),
            strategy: this.calculatePatternAccuracy('strategy', project, detectionResult),
        };

        // Calculate overall metrics
        const overallAccuracy = this.calculateOverallAccuracy(patternAccuracy);
        const falsePositiveRate = this.calculateFalsePositiveRate(patternAccuracy);
        const falseNegativeRate = this.calculateFalseNegativeRate(patternAccuracy);
        const confidenceAccuracy = this.calculateConfidenceAccuracy(project, detectionResult);

        return {
            projectName: project.name,
            overallAccuracy,
            patternAccuracy,
            falsePositiveRate,
            falseNegativeRate,
            confidenceAccuracy,
            processingTime
        };
    }

    /**
     * Calculate accuracy metrics for a specific pattern type
     */
    private calculatePatternAccuracy(
        patternType: 'initialize' | 'authenticate' | 'strategy' | 'route-protection',
        project: GroundTruthProject,
        detectionResult: IntegratedDetectionResult
    ): PatternAccuracyResult {
        const expectedPatterns = project.expectedPatterns.filter(p => p.patternType === patternType);
        const patternKey = patternType;
        const detectedPattern = detectionResult.patterns[patternKey as keyof typeof detectionResult.patterns];
        const detectedEvidence = detectedPattern.evidence;

        let truePositives = 0;
        let falsePositives = 0;
        let falseNegatives = 0;

        const detectedPatterns: string[] = [];
        const missedPatterns: string[] = [];
        const incorrectPatterns: string[] = [];

        // Check each expected pattern
        for (const expected of expectedPatterns) {
            const found = detectedEvidence.find(evidence => 
                this.patternsMatch(expected, evidence)
            );

            if (found) {
                truePositives++;
                detectedPatterns.push(expected.expectedPattern);
            } else {
                falseNegatives++;
                missedPatterns.push(expected.expectedPattern);
            }
        }

        // Check for false positives (detected but not expected)
        for (const detected of detectedEvidence) {
            const expected = expectedPatterns.find((exp: GroundTruthPattern) => 
                this.patternsMatch(exp, detected)
            );

            if (!expected) {
                falsePositives++;
                incorrectPatterns.push(detected.pattern);
            }
        }

        // Calculate metrics
        const precision = truePositives > 0 ? truePositives / (truePositives + falsePositives) : 0;
        const recall = truePositives > 0 ? truePositives / (truePositives + falseNegatives) : 0;
        const f1Score = precision + recall > 0 ? 2 * (precision * recall) / (precision + recall) : 0;

        return {
            truePositives,
            falsePositives,
            falseNegatives,
            precision,
            recall,
            f1Score,
            detectedPatterns,
            missedPatterns,
            incorrectPatterns
        };
    }

    /**
     * Check if detected pattern matches expected pattern
     */
    private patternsMatch(expected: GroundTruthPattern, detected: any): boolean {
        const normalizedExpected = expected.expectedPattern.toLowerCase().replace(/\s+/g, ' ').trim();
        const normalizedDetected = detected.pattern.toLowerCase().replace(/\s+/g, ' ').trim();
        
        // Remove common AST artifacts like "..." and "()"
        const cleanExpected = normalizedExpected.replace(/\(\.\.\.\)/g, '').replace(/\.\.\./g, '');
        const cleanDetected = normalizedDetected.replace(/\(\.\.\.\)/g, '').replace(/\.\.\./g, '');
        
        // Basic substring matching
        if (cleanDetected.includes(cleanExpected) || cleanExpected.includes(cleanDetected)) {
            return true;
        }
        
        // Handle strategy name variations (LocalStrategy vs Strategy)
        if (expected.patternType === 'strategy') {
            // Extract strategy type from expected pattern
            const expectedStrategyMatch = cleanExpected.match(/new (\w+)/);
            const detectedStrategyMatch = cleanDetected.match(/new (\w+)/);
            
            if (expectedStrategyMatch && detectedStrategyMatch) {
                const expectedStrategy = expectedStrategyMatch[1];
                const detectedStrategy = detectedStrategyMatch[1];
                
                // Match LocalStrategy with Strategy, JWTStrategy with Strategy, etc.
                if (expectedStrategy.includes('strategy') && detectedStrategy === 'strategy') {
                    return true;
                }
                if (detectedStrategy.includes('strategy') && expectedStrategy === 'strategy') {
                    return true;
                }
            }
        }
        
        // Handle authenticate method variations
        if (expected.patternType === 'authenticate') {
            const expectedAuthMatch = cleanExpected.match(/passport\.authenticate\s*\(\s*['"]([^'"]+)['"]/);
            const detectedAuthMatch = cleanDetected.match(/passport\.authenticate\s*\(\s*['"]([^'"]+)['"]/);
            
            if (expectedAuthMatch && detectedAuthMatch) {
                // Match if the authentication strategy is the same
                return expectedAuthMatch[1] === detectedAuthMatch[1];
            }
        }
        
        return false;
    }

    /**
     * Calculate overall accuracy from pattern accuracies
     */
    private calculateOverallAccuracy(patternAccuracy: Record<string, PatternAccuracyResult>): number {
        const f1Scores = Object.values(patternAccuracy).map(p => p.f1Score);
        return f1Scores.reduce((sum, score) => sum + score, 0) / f1Scores.length;
    }

    /**
     * Calculate false positive rate
     */
    private calculateFalsePositiveRate(patternAccuracy: Record<string, PatternAccuracyResult>): number {
        const totalFalsePositives = Object.values(patternAccuracy).reduce((sum, p) => sum + p.falsePositives, 0);
        const totalTruePositives = Object.values(patternAccuracy).reduce((sum, p) => sum + p.truePositives, 0);
        
        return totalFalsePositives > 0 ? totalFalsePositives / (totalFalsePositives + totalTruePositives) : 0;
    }

    /**
     * Calculate false negative rate
     */
    private calculateFalseNegativeRate(patternAccuracy: Record<string, PatternAccuracyResult>): number {
        const totalFalseNegatives = Object.values(patternAccuracy).reduce((sum, p) => sum + p.falseNegatives, 0);
        const totalTruePositives = Object.values(patternAccuracy).reduce((sum, p) => sum + p.truePositives, 0);
        
        return totalFalseNegatives > 0 ? totalFalseNegatives / (totalFalseNegatives + totalTruePositives) : 0;
    }

    /**
     * Calculate confidence accuracy (how well confidence scores match reality)
     */
    private calculateConfidenceAccuracy(
        project: GroundTruthProject,
        detectionResult: IntegratedDetectionResult
    ): number {
        // Simplified confidence accuracy - compare predicted confidence with actual accuracy
        const actualAccuracy = detectionResult.overallConfidence / 100;
        const expectedAccuracy = project.expectedPatterns.length > 0 ? 0.8 : 0.1; // Expected based on ground truth
        
        return 1 - Math.abs(actualAccuracy - expectedAccuracy);
    }

    /**
     * Identify common failure patterns across projects
     */
    private identifyCommonFailures(results: AccuracyMetrics[]): string[] {
        const failures: string[] = [];
        
        // Check for patterns with consistently low accuracy
        const patternTypes = ['initialize', 'authenticate', 'strategy'] as const;
        
        for (const patternType of patternTypes) {
            const avgF1 = results.reduce((sum, r) => sum + r.patternAccuracy[patternType].f1Score, 0) / results.length;
            
            if (avgF1 < 0.7) {
                failures.push(`Low ${patternType} detection accuracy (${Math.round(avgF1 * 100)}%)`);
            }
        }

        // Check for high false positive rates
        const avgFPR = results.reduce((sum, r) => sum + r.falsePositiveRate, 0) / results.length;
        if (avgFPR > 0.2) {
            failures.push(`High false positive rate (${Math.round(avgFPR * 100)}%)`);
        }

        return failures;
    }

    /**
     * Generate improvement recommendations based on accuracy results
     */
    private generateRecommendations(
        accuracyByPattern: Record<string, number>,
        commonFailures: string[]
    ): string[] {
        const recommendations: string[] = [];

        // Pattern-specific recommendations
        Object.entries(accuracyByPattern).forEach(([pattern, accuracy]) => {
            if (accuracy < 0.8) {
                switch (pattern) {
                    case 'initialize':
                        recommendations.push('Improve passport.initialize() detection patterns');
                        break;
                    case 'authenticate':
                        recommendations.push('Enhance passport.authenticate() middleware detection');
                        break;
                    case 'strategy':
                        recommendations.push('Expand strategy detection to cover more authentication types');
                        break;
                }
            }
        });

        // General recommendations based on failures
        if (commonFailures.some(f => f.includes('false positive'))) {
            recommendations.push('Implement stricter pattern matching to reduce false positives');
        }

        if (commonFailures.some(f => f.includes('Low') && f.includes('accuracy'))) {
            recommendations.push('Enhance AST parsing to handle more code variations');
        }

        return recommendations;
    }

    /**
     * Generate comprehensive accuracy report
     */
    public generateReport(reports: FrameworkAccuracyReport[]): string {
        let report = '# Accuracy Measurement Report\n\n';
        
        for (const frameworkReport of reports) {
            report += `## ${frameworkReport.framework.toUpperCase()} Framework\n\n`;
            report += `- **Projects Analyzed**: ${frameworkReport.projectsAnalyzed}\n`;
            report += `- **Average Accuracy**: ${Math.round(frameworkReport.averageAccuracy * 100)}%\n\n`;
            
            report += '### Pattern Accuracy:\n';
            Object.entries(frameworkReport.accuracyByPattern).forEach(([pattern, accuracy]) => {
                const percentage = Math.round(accuracy * 100);
                const status = accuracy >= 0.8 ? '✅' : accuracy >= 0.6 ? '⚠️' : '❌';
                report += `- **${pattern}**: ${percentage}% ${status}\n`;
            });
            
            report += '\n### Common Failures:\n';
            frameworkReport.commonFailures.forEach(failure => {
                report += `- ❌ ${failure}\n`;
            });
            
            report += '\n### Recommendations:\n';
            frameworkReport.recommendations.forEach(rec => {
                report += `- 💡 ${rec}\n`;
            });
            
            report += '\n---\n\n';
        }
        
        return report;
    }
}

// Example usage and testing
export async function testAccuracyMeasurement(datasetPath: string = 'test-dataset.json') {
    const framework = new AccuracyMeasurementFramework();
    
    console.log('📊 Testing Accuracy Measurement Framework...\n');
    
    try {
        // Create a sample dataset for testing
        if (!fs.existsSync(datasetPath)) {
            console.log('📝 Creating sample dataset for testing...');
            await createSampleDataset(datasetPath);
        }
        
        framework.loadGroundTruthDataset(datasetPath);
        const reports = await framework.measureAccuracy();
        const reportText = framework.generateReport(reports);
        
        console.log('\n' + reportText);
        
        return reports;
    } catch (error) {
        console.error('❌ Accuracy measurement failed:', error);
        throw error;
    }
}

/**
 * Create a sample dataset for testing
 */
async function createSampleDataset(datasetPath: string) {
    const sampleDataset = {
        version: "1.0",
        created: new Date().toISOString(),
        projects: [
            {
                name: "sample-express-passport",
                path: ".",
                description: "Sample project for testing (current ClaudeCat project)",
                framework: "express",
                authLibrary: "passport",
                expectedPatterns: [
                    // No patterns expected in ClaudeCat project
                ]
            }
        ]
    };
    
    fs.writeFileSync(datasetPath, JSON.stringify(sampleDataset, null, 2));
}