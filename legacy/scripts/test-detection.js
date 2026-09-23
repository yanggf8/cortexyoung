#!/usr/bin/env node

/**
 * Test script to verify pattern detection works correctly
 * Usage: node scripts/test-detection.js [project-path]
 */

import { fileURLToPath } from 'url';
import { dirname, join } from 'path';
import { EnhancedProjectDetector } from '../dist/core/project-detector.js';

const __filename = fileURLToPath(import.meta.url);
const __dirname = dirname(__filename);

async function testPatternDetection() {
  const projectPath = process.argv[2] || process.cwd();
  
  console.log('🧪 Testing Pattern Detection');
  console.log('============================');
  console.log(`Project Path: ${projectPath}\n`);

  try {
    const detector = new EnhancedProjectDetector(projectPath);
    const context = detector.detectCurrentContext();

    // Project basic info
    console.log('📦 Project Information:');
    console.log(`  Type: ${context.projectType}`);
    console.log(`  Language: ${context.language}`);
    console.log(`  Framework: ${context.framework}`);
    console.log(`  Package Manager: ${context.packageManager}`);
    console.log(`  Dependencies: ${context.dependencies.length} found`);
    console.log(`  Directories: ${context.directories.length} mapped\n`);

    // Authentication patterns
    const auth = context.implementationPatterns.authentication;
    console.log('🔐 Authentication Patterns:');
    console.log(`  Confidence: ${auth.confidence}%`);
    console.log(`  User Property: ${auth.userProperty}`);
    console.log(`  Token Location: ${auth.tokenLocation}`);
    console.log(`  Error Format: ${auth.errorResponse.format}`);
    console.log(`  Status Code: ${auth.errorResponse.statusCode}`);
    console.log(`  Middleware: ${auth.middlewarePattern}`);
    if (auth.evidence.length > 0) {
      console.log(`  Evidence: ${auth.evidence.slice(0, 2).join(', ')}`);
    }
    console.log();

    // API response patterns
    const api = context.implementationPatterns.apiResponses;
    console.log('🌐 API Response Patterns:');
    console.log(`  Confidence: ${api.confidence}%`);
    console.log(`  Success Format: ${api.successFormat}`);
    console.log(`  Error Format: ${api.errorFormat}`);
    console.log(`  Status Codes: ${api.statusCodeUsage}`);
    console.log(`  Wrapper Pattern: ${api.wrapperPattern}`);
    if (api.evidence.length > 0) {
      console.log(`  Evidence: ${api.evidence.slice(0, 2).join(', ')}`);
    }
    console.log();

    // Error handling patterns
    const errors = context.implementationPatterns.errorHandling;
    console.log('⚠️  Error Handling Patterns:');
    console.log(`  Confidence: ${errors.confidence}%`);
    console.log(`  Catch Pattern: ${errors.catchPattern}`);
    console.log(`  Error Structure: ${errors.errorStructure}`);
    console.log(`  Logging: ${errors.loggingIntegration}`);
    console.log(`  Propagation: ${errors.propagationStyle}`);
    if (errors.evidence.length > 0) {
      console.log(`  Evidence: ${errors.evidence.slice(0, 2).join(', ')}`);
    }
    console.log();

    // Overall assessment
    const avgConfidence = Math.round(
      (auth.confidence + api.confidence + errors.confidence) / 3
    );
    
    console.log('📊 Detection Summary:');
    console.log(`  Average Confidence: ${avgConfidence}%`);
    
    if (avgConfidence >= 80) {
      console.log('  Status: ✅ High confidence - patterns clearly detected');
    } else if (avgConfidence >= 60) {
      console.log('  Status: ⚠️  Medium confidence - some patterns detected');
    } else {
      console.log('  Status: ❌ Low confidence - patterns unclear');
    }

    // Generate sample guardrails
    console.log('\n🛡️  Generated Guardrails:');
    
    if (auth.tokenLocation === 'httpOnly cookie' && auth.confidence >= 60) {
      console.log('  ❌ NEVER use localStorage for tokens - Project uses httpOnly cookies');
    }
    if (auth.userProperty !== 'Unknown' && auth.confidence >= 60) {
      console.log(`  ✅ ALWAYS use ${auth.userProperty} for authenticated user data`);
    }
    if (api.successFormat !== 'Unknown' && api.confidence >= 60) {
      if (api.successFormat === 'bare object') {
        console.log('  ✅ Use bare object responses - No wrapper format detected');
      } else {
        console.log(`  ✅ ALWAYS wrap API responses in ${api.successFormat} format`);
      }
    }

    if (avgConfidence < 60) {
      console.log('  ⚠️  Implementation patterns unclear - Ask before making assumptions');
    }

    console.log('\n✅ Pattern detection test completed');

  } catch (error) {
    console.error('❌ Pattern detection failed:', error.message);
    process.exit(1);
  }
}

// Run the test
testPatternDetection().catch(error => {
  console.error('💥 Test script failed:', error);
  process.exit(1);
});