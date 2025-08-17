#!/usr/bin/env node

/**
 * Debug script to reproduce the exact bug where src/indexer.ts
 * is being marked as "deleted" instead of "modified" in delta detection
 */

const { GitScanner } = require('./dist/git-scanner');
const path = require('path');

async function reproduceIssue() {
  console.log('🔍 Debugging Delta Detection Bug');
  console.log('='.repeat(50));
  
  const repoPath = process.cwd();
  const gitScanner = new GitScanner(repoPath);
  
  console.log(`📂 Repository path: ${repoPath}`);
  console.log();
  
  // Step 1: Test git ls-files directly
  console.log('📋 Step 1: Testing git ls-files directly');
  const { spawn } = require('child_process');
  const gitLsFiles = spawn('git', ['ls-files'], { cwd: repoPath });
  
  let gitOutput = '';
  gitLsFiles.stdout.on('data', (data) => {
    gitOutput += data.toString();
  });
  
  await new Promise((resolve) => {
    gitLsFiles.on('close', resolve);
  });
  
  const gitFiles = gitOutput.split('\n').filter(Boolean);
  const indexerInGit = gitFiles.find(f => f.includes('src/indexer.ts'));
  console.log(`   git ls-files includes src/indexer.ts: ${!!indexerInGit}`);
  console.log(`   Total files from git ls-files: ${gitFiles.length}`);
  console.log();
  
  // Step 2: Test GitScanner.scanRepository('full')
  console.log('📋 Step 2: Testing GitScanner.scanRepository("full")');
  try {
    const scanResult = await gitScanner.scanRepository('full');
    const indexerInScan = scanResult.files.find(f => f.includes('src/indexer.ts'));
    
    console.log(`   GitScanner includes src/indexer.ts: ${!!indexerInScan}`);
    console.log(`   Total files from GitScanner: ${scanResult.totalFiles}`);
    console.log(`   Files array length: ${scanResult.files.length}`);
    
    if (!indexerInScan) {
      console.log('❌ BUG IDENTIFIED: src/indexer.ts is missing from GitScanner results!');
      console.log();
      console.log('   Files that DO include "src/":', scanResult.files.filter(f => f.includes('src/')).slice(0, 10));
      console.log('   Files that include "indexer":', scanResult.files.filter(f => f.includes('indexer')));
    } else {
      console.log('✅ src/indexer.ts is correctly included in GitScanner results');
    }
    
    console.log();
    
    // Step 3: Test file existence check (filterExistingFiles equivalent)
    console.log('📋 Step 3: Testing file existence check');
    const fs = require('fs/promises');
    const fullPath = path.join(repoPath, 'src/indexer.ts');
    
    try {
      await fs.access(fullPath, fs.constants.F_OK);
      console.log(`   ✅ File exists at: ${fullPath}`);
    } catch (error) {
      console.log(`   ❌ File access failed: ${error.message}`);
    }
    
    console.log();
    
    // Step 4: Test isRelevantFile logic
    console.log('📋 Step 4: Testing isRelevantFile logic');
    const filePath = 'src/indexer.ts';
    const ext = path.extname(filePath).toLowerCase();
    const relevantExtensions = [
      '.js', '.jsx', '.ts', '.tsx',
      '.py', '.go', '.rs', '.java',
      '.cpp', '.c', '.h', '.hpp',
      '.md', '.rst', '.txt',
      '.json', '.yaml', '.yml', '.toml',
      '.html', '.css', '.scss'
    ];
    const skipDirs = ['node_modules', '.git', 'dist', 'build', 'coverage', '.next', '.cache'];
    
    const hasRelevantExt = relevantExtensions.includes(ext);
    const hasSkipDir = skipDirs.some(dir => filePath.includes(dir));
    const isRelevant = hasRelevantExt && !hasSkipDir;
    
    console.log(`   File extension: ${ext}`);
    console.log(`   Has relevant extension: ${hasRelevantExt}`);
    console.log(`   Has skip directory: ${hasSkipDir}`);
    console.log(`   Is relevant file: ${isRelevant}`);
    
  } catch (error) {
    console.error('❌ Error in GitScanner test:', error.message);
  }
}

reproduceIssue().catch(console.error);