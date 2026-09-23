#!/usr/bin/env node

/**
 * Comprehensive accuracy testing script for ClaudeCat
 * 
 * Tests pattern detection accuracy against ground truth dataset
 * and generates detailed accuracy reports.
 */

import { testAccuracyMeasurement } from '../dist/accuracy-measurement-framework.js';
import fs from 'fs';

async function main() {
    console.log('🚀 ClaudeCat Accuracy Testing Suite');
    console.log('=====================================\n');
    
    const datasetPath = 'ground-truth-dataset.json';
    
    if (!fs.existsSync(datasetPath)) {
        console.error(`❌ Dataset not found: ${datasetPath}`);
        console.log('📝 Please ensure ground-truth-dataset.json exists');
        process.exit(1);
    }
    
    try {
        console.log('📊 Starting comprehensive accuracy measurement...\n');
        const reports = await testAccuracyMeasurement(datasetPath);
        
        console.log('\n✅ Accuracy testing completed successfully!');
        console.log(`📈 Analyzed ${reports.reduce((sum, r) => sum + r.projectsAnalyzed, 0)} projects`);
        console.log(`🎯 Average accuracy: ${Math.round(reports.reduce((sum, r) => sum + r.averageAccuracy, 0) / reports.length * 100)}%`);
        
        // Save detailed report
        const timestamp = new Date().toISOString().replace(/[:.]/g, '-');
        const reportFile = `accuracy-report-${timestamp}.md`;
        
        let fullReport = '# ClaudeCat Accuracy Test Report\n\n';
        fullReport += `**Test Date**: ${new Date().toISOString()}\n\n`;
        
        for (const report of reports) {
            fullReport += `## ${report.framework.toUpperCase()} Framework Results\n\n`;
            fullReport += `- **Projects Tested**: ${report.projectsAnalyzed}\n`;
            fullReport += `- **Overall Accuracy**: ${Math.round(report.averageAccuracy * 100)}%\n\n`;
            
            fullReport += '### Pattern Detection Results:\n';
            Object.entries(report.accuracyByPattern).forEach(([pattern, accuracy]) => {
                const percentage = Math.round(accuracy * 100);
                const status = accuracy >= 0.95 ? '✅ Excellent' : 
                              accuracy >= 0.80 ? '✅ Good' :
                              accuracy >= 0.60 ? '⚠️ Needs Improvement' : 
                              '❌ Poor';
                fullReport += `- **${pattern}**: ${percentage}% ${status}\n`;
            });
            
            if (report.commonFailures.length > 0) {
                fullReport += '\n### Issues Identified:\n';
                report.commonFailures.forEach(failure => {
                    fullReport += `- 🔍 ${failure}\n`;
                });
            }
            
            if (report.recommendations.length > 0) {
                fullReport += '\n### Improvement Recommendations:\n';
                report.recommendations.forEach(rec => {
                    fullReport += `- 💡 ${rec}\n`;
                });
            }
            
            fullReport += '\n---\n\n';
        }
        
        fs.writeFileSync(reportFile, fullReport);
        console.log(`📄 Detailed report saved: ${reportFile}`);
        
    } catch (error) {
        console.error('❌ Accuracy testing failed:', error);
        process.exit(1);
    }
}

main();