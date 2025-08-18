const { ContentAnalyzer } = require('./dist/content-analyzer');
const fs = require('fs/promises');
const path = require('path');

async function testContentAnalyzer() {
  console.log('🧪 Testing Content Analyzer...');
  
  const analyzer = new ContentAnalyzer();
  
  // Test with actual project files
  const testFiles = [
    'src/server.ts',
    'src/indexer.ts', 
    'package.json',
    'README.md',
    'src/types.ts'
  ];
  
  console.log('\n📊 Analyzing sample files:');
  console.log('=' .repeat(80));
  
  for (const filePath of testFiles) {
    try {
      const analysis = await analyzer.analyzeFile(filePath);
      
      console.log(`\n📁 ${filePath}`);
      console.log(`   Language: ${analysis.language}`);
      console.log(`   File Type: ${analysis.fileType}`);
      console.log(`   Semantic Value: ${analysis.semanticValue}`);
      console.log(`   Importance Score: ${analysis.estimatedImportance}/100`);
      console.log(`   Complexity: ${analysis.complexity}`);
      console.log(`   Has Imports: ${analysis.hasImports ? '✅' : '❌'}`);
      console.log(`   Has Exports: ${analysis.hasExports ? '✅' : '❌'}`);
      console.log(`   Has Tests: ${analysis.hasTests ? '✅' : '❌'}`);
      console.log(`   Has Documentation: ${analysis.hasDocumentation ? '✅' : '❌'}`);
      console.log(`   Code/Comment Ratio: ${analysis.codeToCommentRatio.toFixed(2)}`);
      console.log(`   Unique Tokens: ${analysis.uniqueTokens}`);
      
    } catch (error) {
      console.log(`❌ Failed to analyze ${filePath}: ${error.message}`);
    }
  }
  
  // Test batch analysis
  console.log('\n🔄 Testing batch analysis...');
  const allFiles = await getAllProjectFiles();
  const batchResults = await analyzer.analyzeFiles(allFiles.slice(0, 20)); // Test first 20 files
  
  console.log(`\n📈 Batch Analysis Results (${batchResults.size} files):`);
  
  // Language distribution
  const languageStats = analyzer.getLanguageStats(batchResults);
  console.log('\n🗣️  Language Distribution:');
  for (const [language, count] of Object.entries(languageStats)) {
    console.log(`   ${language}: ${count} files`);
  }
  
  // Semantic value distribution
  const semanticStats = analyzer.getSemanticValueDistribution(batchResults);
  console.log('\n🎯 Semantic Value Distribution:');
  console.log(`   High: ${semanticStats.high} files`);
  console.log(`   Medium: ${semanticStats.medium} files`);
  console.log(`   Low: ${semanticStats.low} files`);
  
  // Top importance files
  const sortedByImportance = Array.from(batchResults.entries())
    .sort(([,a], [,b]) => b.estimatedImportance - a.estimatedImportance)
    .slice(0, 10);
    
  console.log('\n⭐ Top 10 Most Important Files:');
  sortedByImportance.forEach(([filePath, analysis], index) => {
    console.log(`   ${index + 1}. ${filePath} (${analysis.estimatedImportance}/100)`);
  });
  
  // Performance test
  console.log('\n⚡ Performance Test...');
  const startTime = Date.now();
  await analyzer.analyzeFiles(allFiles.slice(0, 50));
  const endTime = Date.now();
  console.log(`   Analyzed 50 files in ${endTime - startTime}ms`);
  console.log(`   Average: ${((endTime - startTime) / 50).toFixed(1)}ms per file`);
  
  console.log('\n✅ Content Analyzer test completed!');
}

async function getAllProjectFiles() {
  const files = [];
  
  async function scanDir(dir) {
    try {
      const entries = await fs.readdir(dir, { withFileTypes: true });
      
      for (const entry of entries) {
        const fullPath = path.join(dir, entry.name);
        
        if (entry.isDirectory()) {
          // Skip common ignore directories
          if (!['node_modules', '.git', 'dist', 'build', '.cortex'].includes(entry.name)) {
            await scanDir(fullPath);
          }
        } else if (entry.isFile()) {
          // Include common source file types
          const ext = path.extname(entry.name);
          if (['.ts', '.js', '.md', '.json', '.yml', '.yaml'].includes(ext)) {
            files.push(fullPath);
          }
        }
      }
    } catch (error) {
      // Skip directories we can't read
    }
  }
  
  await scanDir('.');
  return files;
}

if (require.main === module) {
  testContentAnalyzer().catch(console.error);
}

module.exports = { testContentAnalyzer };