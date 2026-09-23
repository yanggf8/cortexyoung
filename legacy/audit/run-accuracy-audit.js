#!/usr/bin/env node

/**
 * ClaudeCat Accuracy Self-Audit
 * Tests if our implementation actually delivers Claude Code accuracy improvements
 */

import { EnhancedProjectDetector } from '../dist/core/project-detector.js';
import path from 'path';
import fs from 'fs';
import { fileURLToPath } from 'url';

const __filename = fileURLToPath(import.meta.url);
const __dirname = path.dirname(__filename);

class AccuracyAuditor {
  constructor() {
    this.results = {
      patternDetection: [],
      falsePositives: [],
      confidenceCalibration: [],
      integrationTests: []
    };
  }

  async runFullAudit() {
    console.log('🔍 Starting ClaudeCat Accuracy Self-Audit\n');
    
    // Phase 1: Pattern Detection Accuracy
    console.log('📊 Phase 1: Pattern Detection Accuracy');
    await this.testPatternDetection();
    
    // Phase 2: False Positive Detection
    console.log('\n🚫 Phase 2: False Positive Detection');  
    await this.testFalsePositives();
    
    // Phase 3: Confidence Calibration
    console.log('\n📈 Phase 3: Confidence Calibration');
    await this.testConfidenceCalibration();
    
    // Phase 4: Integration Pattern Validation
    console.log('\n🔗 Phase 4: Integration Pattern Validation');
    await this.testIntegrationPatterns();
    
    // Generate Report
    console.log('\n📋 Generating Audit Report');
    this.generateReport();
  }

  async testPatternDetection() {
    const testProjects = [
      {
        name: 'Project A (Standard Express)',
        path: path.join(__dirname, 'test-project-a'),
        expected: {
          userProperty: 'req.user',
          tokenLocation: 'httpOnly cookie',
          responseWrapper: '{data: any}',
          errorHandling: 'global middleware'
        }
      },
      {
        name: 'Project B (Alternative Patterns)', 
        path: path.join(__dirname, 'test-project-b'),
        expected: {
          userProperty: 'req.context.user',
          tokenLocation: 'JWT headers',
          responseWrapper: 'bare objects',
          errorHandling: 'try/catch blocks'
        }
      }
    ];

    for (const project of testProjects) {
      console.log(`  Testing: ${project.name}`);
      
      if (!fs.existsSync(project.path)) {
        console.log(`    ❌ Project path not found: ${project.path}`);
        continue;
      }

      const detector = new EnhancedProjectDetector(project.path);
      const context = detector.detectCurrentContext();
      
      const result = this.validateDetectedPatterns(context, project.expected);
      this.results.patternDetection.push({
        project: project.name,
        expected: project.expected,
        detected: {
          userProperty: context.implementationPatterns?.authentication?.userProperty || 'Unknown',
          tokenLocation: context.implementationPatterns?.authentication?.tokenLocation || 'Unknown',
          responseWrapper: context.implementationPatterns?.apiResponses?.successFormat || 'Unknown',
          errorHandling: context.implementationPatterns?.errorHandling?.catchPattern || 'Unknown'
        },
        accuracy: result.accuracy,
        details: result.details
      });
      
      console.log(`    Accuracy: ${result.accuracy}% (${result.details})`);
    }
  }

  async testFalsePositives() {
    console.log('  Testing false positive detection...');
    
    const falsePositiveFile = path.join(__dirname, 'false-positive-test.js');
    if (!fs.existsSync(falsePositiveFile)) {
      console.log('    ❌ False positive test file not found');
      return;
    }

    // Create temporary directory with false positive file
    const tempDir = path.join(__dirname, 'temp-false-positive-test');
    if (!fs.existsSync(tempDir)) {
      fs.mkdirSync(tempDir);
    }
    
    // Copy false positive file to temp directory
    fs.copyFileSync(falsePositiveFile, path.join(tempDir, 'server.js'));
    
    const detector = new EnhancedProjectDetector(tempDir);
    const context = detector.detectCurrentContext();
    
    // Should detect NO patterns or very low confidence
    const falsePositiveCount = this.countDetectedPatterns(context);
    const avgConfidence = this.calculateAverageConfidence(context);
    
    this.results.falsePositives.push({
      patternsDetected: falsePositiveCount,
      averageConfidence: avgConfidence,
      passed: falsePositiveCount === 0 || avgConfidence < 30
    });
    
    console.log(`    Patterns detected: ${falsePositiveCount} (should be 0)`);
    console.log(`    Average confidence: ${avgConfidence}% (should be <30%)`);
    console.log(`    Test ${falsePositiveCount === 0 || avgConfidence < 30 ? '✅ PASSED' : '❌ FAILED'}`);
    
    // Cleanup
    fs.rmSync(tempDir, { recursive: true });
  }

  async testConfidenceCalibration() {
    console.log('  Testing confidence score accuracy...');
    
    // Test with current project (should have some patterns)
    const detector = new EnhancedProjectDetector(process.cwd());
    const context = detector.detectCurrentContext();
    
    const patterns = context.implementationPatterns;
    const calibrationTest = {
      authConfidence: patterns?.authentication?.confidence?.totalConfidence || 0,
      apiConfidence: patterns?.apiResponses?.confidence?.totalConfidence || 0,
      errorConfidence: patterns?.errorHandling?.confidence?.totalConfidence || 0
    };
    
    this.results.confidenceCalibration.push(calibrationTest);
    
    console.log(`    Auth patterns: ${calibrationTest.authConfidence}% confidence`);
    console.log(`    API patterns: ${calibrationTest.apiConfidence}% confidence`);  
    console.log(`    Error patterns: ${calibrationTest.errorConfidence}% confidence`);
  }

  async testIntegrationPatterns() {
    console.log('  Testing pattern integration consistency...');
    
    const detector = new EnhancedProjectDetector(process.cwd());
    const context = detector.detectCurrentContext();
    
    // Check if patterns are consistent with each other
    const integration = this.validatePatternIntegration(context.implementationPatterns);
    this.results.integrationTests.push(integration);
    
    console.log(`    Pattern consistency: ${integration.consistent ? '✅ GOOD' : '⚠️ INCONSISTENT'}`);
    if (integration.issues.length > 0) {
      integration.issues.forEach(issue => console.log(`      - ${issue}`));
    }
  }

  validateDetectedPatterns(context, expected) {
    let correct = 0;
    let total = 0;
    const details = [];

    const patterns = context.implementationPatterns;
    
    // Check authentication pattern
    if (patterns?.authentication?.userProperty === expected.userProperty) correct++;
    else details.push(`Expected ${expected.userProperty}, got ${patterns?.authentication?.userProperty || 'Unknown'}`);
    total++;
    
    // Add more pattern validations...
    
    return {
      accuracy: Math.round((correct / total) * 100),
      details: details.join('; ') || 'All patterns correct'
    };
  }

  countDetectedPatterns(context) {
    let count = 0;
    const patterns = context.implementationPatterns;
    
    if (patterns?.authentication?.userProperty && patterns.authentication.userProperty !== 'Unknown') count++;
    if (patterns?.apiResponses?.successFormat && patterns.apiResponses.successFormat !== 'Unknown') count++;
    if (patterns?.errorHandling?.catchPattern && patterns.errorHandling.catchPattern !== 'Unknown') count++;
    
    return count;
  }

  calculateAverageConfidence(context) {
    const confidences = [];
    const patterns = context.implementationPatterns;
    
    if (patterns?.authentication?.confidence?.totalConfidence) {
      confidences.push(patterns.authentication.confidence.totalConfidence);
    }
    if (patterns?.apiResponses?.confidence?.totalConfidence) {
      confidences.push(patterns.apiResponses.confidence.totalConfidence);
    }
    if (patterns?.errorHandling?.confidence?.totalConfidence) {
      confidences.push(patterns.errorHandling.confidence.totalConfidence);
    }
    
    return confidences.length > 0 ? Math.round(confidences.reduce((a, b) => a + b) / confidences.length) : 0;
  }

  validatePatternIntegration(patterns) {
    const issues = [];
    let consistent = true;

    // Example integration checks
    if (patterns?.authentication?.tokenLocation === 'httpOnly cookie' && 
        patterns?.authentication?.userProperty === 'req.context.user') {
      issues.push('Inconsistent: httpOnly cookies typically use req.user, not req.context.user');
      consistent = false;
    }

    return { consistent, issues };
  }

  generateReport() {
    console.log('\n' + '='.repeat(60));
    console.log('📋 CLAUDECAT ACCURACY AUDIT REPORT');
    console.log('='.repeat(60));
    
    // Pattern Detection Results
    console.log('\n🎯 PATTERN DETECTION ACCURACY:');
    const avgAccuracy = this.results.patternDetection.reduce((sum, r) => sum + r.accuracy, 0) / this.results.patternDetection.length;
    console.log(`  Average Accuracy: ${avgAccuracy.toFixed(1)}%`);
    
    this.results.patternDetection.forEach(result => {
      console.log(`  ${result.project}: ${result.accuracy}%`);
    });
    
    // False Positive Results
    console.log('\n🚫 FALSE POSITIVE DETECTION:');
    const fpResults = this.results.falsePositives[0];
    if (fpResults) {
      console.log(`  Patterns detected in comments/docs: ${fpResults.patternsDetected}`);
      console.log(`  Average confidence: ${fpResults.averageConfidence}%`);
      console.log(`  Test result: ${fpResults.passed ? '✅ PASSED' : '❌ FAILED'}`);
    }
    
    // Overall Assessment
    console.log('\n🏆 OVERALL ASSESSMENT:');
    
    if (avgAccuracy >= 95 && fpResults?.passed) {
      console.log('  ✅ EXCELLENT - Ready for production accuracy improvements');
    } else if (avgAccuracy >= 85) {
      console.log('  ⚠️ GOOD - Some accuracy concerns, but usable');
    } else {
      console.log('  ❌ POOR - Significant accuracy issues, needs improvement');
    }
    
    console.log('\n📊 RECOMMENDATIONS:');
    if (avgAccuracy < 95) {
      console.log('  • Improve pattern detection algorithms');
      console.log('  • Add more validation rules');
    }
    if (!fpResults?.passed) {
      console.log('  • Strengthen context validation to reduce false positives');
      console.log('  • Raise confidence thresholds');
    }
    if (avgAccuracy >= 95 && fpResults?.passed) {
      console.log('  • Pattern detection is working well!');
      console.log('  • Consider testing on more diverse codebases');
    }
  }
}

// Run the audit
if (import.meta.url === `file://${process.argv[1]}`) {
  const auditor = new AccuracyAuditor();
  auditor.runFullAudit().catch(console.error);
}

export default AccuracyAuditor;