#!/usr/bin/env node

import { EnhancedProjectDetector } from '../dist/core/project-detector.js';
import { promises as fs } from 'fs';
import { join } from 'path';

/**
 * Test script for validating ClaudeCat improvements across real projects
 * 
 * Usage: 
 * 1. Clone/download 10+ real Node.js projects to a test directory
 * 2. Run: node scripts/test-real-projects.js /path/to/test/projects/directory
 */

const testProjects = [
  // Express.js projects
  'express-api-starter',
  'express-mongodb-api',
  'express-auth-example',
  
  // Next.js projects  
  'nextjs-blog',
  'nextjs-ecommerce',
  
  // Fastify projects
  'fastify-api',
  
  // NestJS projects
  'nestjs-starter',
  
  // Other Node.js projects
  'node-typescript-api',
  'koa-rest-api',
  'hapi-api-boilerplate'
];

async function testRealProjects(baseDir = process.argv[2]) {
  if (!baseDir) {
    console.error('❌ Usage: node scripts/test-real-projects.js /path/to/test/projects/directory');
    console.error('\nExample test projects to download:');
    console.error('  - https://github.com/hagopj13/node-express-boilerplate');
    console.error('  - https://github.com/danielfsousa/express-rest-es2017-boilerplate');
    console.error('  - https://github.com/vercel/next.js/tree/canary/examples/api-routes');
    console.error('  - https://github.com/nestjs/nest-starter');
    console.error('  - https://github.com/fastify/fastify-example');
    process.exit(1);
  }

  console.log('🚀 Testing ClaudeCat accuracy on real projects...\n');
  console.log(`Base directory: ${baseDir}\n`);

  const results = [];
  let totalProjects = 0;
  let successfulDetections = 0;

  try {
    const entries = await fs.readdir(baseDir);
    const projectDirs = [];
    
    for (const entry of entries) {
      const fullPath = join(baseDir, entry);
      const stat = await fs.stat(fullPath);
      if (stat.isDirectory()) {
        // Check if it's a Node.js project
        try {
          await fs.access(join(fullPath, 'package.json'));
          projectDirs.push({ name: entry, path: fullPath });
        } catch {
          // Not a Node.js project, skip
        }
      }
    }

    console.log(`📁 Found ${projectDirs.length} Node.js projects to test:\n`);

    for (const project of projectDirs) {
      totalProjects++;
      console.log(`\n${'='.repeat(60)}`);
      console.log(`🔍 Testing: ${project.name}`);
      console.log(`${'='.repeat(60)}`);

      try {
        const detector = new EnhancedProjectDetector(project.path);
        const result = detector.detectCurrentContext();
        
        // Analyze results
        const authConfidence = result.implementationPatterns.authentication.confidence;
        const apiConfidence = result.implementationPatterns.apiResponses.confidence;  
        const errorConfidence = result.implementationPatterns.errorHandling.confidence;
        const avgConfidence = (authConfidence + apiConfidence + errorConfidence) / 3;

        results.push({
          name: project.name,
          projectType: result.projectType,
          framework: result.framework,
          language: result.language,
          authConfidence,
          apiConfidence,
          errorConfidence,
          avgConfidence,
          mainFiles: result.mainFiles?.length || 0,
          authEvidence: result.implementationPatterns.authentication.evidence.length,
          apiEvidence: result.implementationPatterns.apiResponses.evidence.length,
          errorEvidence: result.implementationPatterns.errorHandling.evidence.length
        });

        console.log(`✅ Project Type: ${result.projectType}`);
        console.log(`✅ Framework: ${result.framework}`);
        console.log(`✅ Language: ${result.language}`);
        console.log(`✅ Main Files: ${result.mainFiles?.length || 0} detected`);
        
        console.log(`\n📊 Pattern Detection Confidence:`);
        console.log(`  🔐 Auth: ${authConfidence}% (${result.implementationPatterns.authentication.evidence.length} evidence)`);
        console.log(`  📡 API: ${apiConfidence}% (${result.implementationPatterns.apiResponses.evidence.length} evidence)`);  
        console.log(`  ⚠️  Error: ${errorConfidence}% (${result.implementationPatterns.errorHandling.evidence.length} evidence)`);
        console.log(`  📈 Average: ${avgConfidence.toFixed(1)}%`);

        if (avgConfidence >= 60) {
          successfulDetections++;
          console.log(`✅ PASSED - Good confidence detection`);
        } else {
          console.log(`⚠️  PARTIAL - Low confidence, may need manual verification`);
        }

      } catch (error) {
        console.log(`❌ FAILED - Error: ${error.message}`);
        results.push({
          name: project.name,
          error: error.message,
          avgConfidence: 0
        });
      }
    }

  } catch (error) {
    console.error(`❌ Error reading test directory: ${error.message}`);
    process.exit(1);
  }

  // Generate summary report
  console.log(`\n${'='.repeat(80)}`);
  console.log('📊 SUMMARY REPORT');
  console.log(`${'='.repeat(80)}`);
  
  console.log(`\n📈 Overall Results:`);
  console.log(`  Total Projects Tested: ${totalProjects}`);
  console.log(`  Successful Detections (≥60% confidence): ${successfulDetections}`);
  console.log(`  Success Rate: ${((successfulDetections / totalProjects) * 100).toFixed(1)}%`);
  
  const validResults = results.filter(r => !r.error);
  if (validResults.length > 0) {
    const avgAuthConfidence = validResults.reduce((sum, r) => sum + (r.authConfidence || 0), 0) / validResults.length;
    const avgApiConfidence = validResults.reduce((sum, r) => sum + (r.apiConfidence || 0), 0) / validResults.length;
    const avgErrorConfidence = validResults.reduce((sum, r) => sum + (r.errorConfidence || 0), 0) / validResults.length;
    
    console.log(`\n📊 Average Confidence Scores:`);
    console.log(`  🔐 Authentication: ${avgAuthConfidence.toFixed(1)}%`);
    console.log(`  📡 API Responses: ${avgApiConfidence.toFixed(1)}%`);
    console.log(`  ⚠️  Error Handling: ${avgErrorConfidence.toFixed(1)}%`);
  }

  console.log(`\n🏆 Top Performing Projects:`);
  const topProjects = validResults
    .sort((a, b) => (b.avgConfidence || 0) - (a.avgConfidence || 0))
    .slice(0, 5);
    
  topProjects.forEach((project, i) => {
    console.log(`  ${i + 1}. ${project.name}: ${(project.avgConfidence || 0).toFixed(1)}% confidence`);
  });

  if (successfulDetections >= 8) {
    console.log(`\n🎉 SUCCESS: ClaudeCat improvements validated!`);
    console.log(`   Achieved ${((successfulDetections / totalProjects) * 100).toFixed(1)}% success rate on real projects`);
  } else {
    console.log(`\n⚠️  NEEDS IMPROVEMENT: Only ${successfulDetections}/${totalProjects} projects had good detection`);
    console.log(`   Consider further refinements to pattern detection algorithms`);
  }
  
  console.log(`\n📝 Detailed results saved to: test-results-${Date.now()}.json`);
  await fs.writeFile(
    `test-results-${Date.now()}.json`, 
    JSON.stringify(results, null, 2)
  );
}

testRealProjects().catch(console.error);