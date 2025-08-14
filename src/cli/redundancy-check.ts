#!/usr/bin/env node

import { RedundancyChecker } from '../redundancy-checker';
import { PersistentVectorStore } from '../persistent-vector-store';
import { PersistentRelationshipStore } from '../persistent-relationship-store';
import { existsSync, mkdirSync, writeFileSync } from 'fs';

async function main() {
  console.log('🔍 Cortex Redundancy Checker');
  console.log('================================\n');

  const vectorStore = new PersistentVectorStore(process.cwd());
  const relationshipStore = new PersistentRelationshipStore(process.cwd());
  
  try {
    await vectorStore.initialize();
    await relationshipStore.initialize();
    
    const checker = new RedundancyChecker(vectorStore, relationshipStore);
    
    // Parse command line arguments
    const args = process.argv.slice(2);
    const type = args.find(arg => arg.startsWith('--type='))?.split('=')[1];
    const cleanup = args.includes('--cleanup');
    const output = args.find(arg => arg.startsWith('--output='))?.split('=')[1];
    
    let report = '';
    
    if (!type || type === 'full') {
      report = await checker.generateReport();
    } else if (type === 'vector') {
      const rpt = await checker.checkVectorRedundancy();
      report = `# Vector Redundancy Report

Total Items: ${rpt.totalItems}
Duplicates: ${rpt.duplicateCount}
Space Wasted: ${rpt.spaceSaved} bytes

### Duplicates
${rpt.duplicates.map((d: any) => 
  '* ID: ' + d.id + ' (Count: ' + d.count + ')').join('\n')}`;

      if (cleanup && rpt.duplicateCount > 0) {
        console.log('🧹 Cleaning vector duplicates...');
        const cleaned = await checker.cleanupRedundancy(rpt);
        console.log(`✅ Cleaned ${cleaned} duplicate vectors`);
      }
    } else if (type === 'relationship') {
      const rpt = await checker.checkRelationshipRedundancy();
      report = `# Relationship Redundancy Report

Total Relationships: ${rpt.totalItems}
Duplicates: ${rpt.duplicateCount}
Space Wasted: ${rpt.spaceSaved} bytes

### Duplicates
${rpt.duplicates.map((d: any) => 
  '* ID: ' + d.id + ' (Count: ' + d.count + ')').join('\n')}`;

      if (cleanup && rpt.duplicateCount > 0) {
        console.log('🧹 Cleaning relationship duplicates...');
        const cleaned = await checker.cleanupRedundancy(rpt);
        console.log(`✅ Cleaned ${cleaned} duplicate relationships`);
      }
    } else if (type === 'process') {
      const rpt = await checker.checkProcessRedundancy();
      report = `# Process Redundancy Report

Total Process: ${rpt.totalItems}
Duplicates: ${rpt.duplicateCount}
Space Wasted: ${rpt.spaceSaved} bytes

### Duplicates
${rpt.duplicates.map((d: any) => 
  '* ' + d.id.substring(0, 50) + '... (Count: ' + d.count + ')').join('\n')}`;
    } else if (type === 'code') {
      const rpt = await checker.checkCodeRedundancy();
      report = `# Code Redundancy Report

Total Lines: ${rpt.totalItems}
Duplicates: ${rpt.duplicateCount}
Space Wasted: ${rpt.spaceSaved} bytes

### Duplicates
${rpt.duplicates.slice(0, 5).map((d: any) => 
  '* File: ' + d.id + ' (Count: ' + d.count + ')').join('\n')}`;
    }
    
    if (output) {
      const outputPath = output.startsWith('/') ? output : 
        './reports/' + output;
      
      const outputDir = outputPath.substring(0, outputPath.lastIndexOf('/'));
      if (!existsSync(outputDir)) {
        mkdirSync(outputDir, { recursive: true });
      }
      
      writeFileSync(outputPath, report);
      console.log(`✅ Report saved to ${outputPath}`);
    } else {
      console.log(report);
    }
    
  } catch (error) {
    console.error('❌ Error running redundancy check:', error);
    process.exit(1);
  } finally {
    await vectorStore.close();
    await relationshipStore.close();
  }
}

// Handle CLI commands
if (require.main === module) {
  main().catch(error => {
    console.error(error);
    process.exit(1);
  });
}

export { main };