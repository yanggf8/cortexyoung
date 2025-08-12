# 🔍 Redundancy Check System

This system provides comprehensive redundancy detection and cleanup for the Cortex system, handling duplicates in vectors, relationships, processes, and code.

## Quick Start

### 1. Full Redundancy Check
```bash
npm run redundancy:check
```

### 2. Specific Type Checks
```bash
# Vector duplicates
npm run redundancy:check:vector

# Relationship duplicates  
npm run redundancy:check:relationship

# Process duplicates
npm run redundancy:check:process

# Code duplicates
npm run redundancy:check:code
```

### 3. Cleanup Operations
```bash
# Clean vector duplicates
npm run redundancy:cleanup:vector

# Clean relationship duplicates
npm run redundancy:cleanup:relationship
```

### 4. Advanced Validation
```bash
# Validate and clean relationship duplicates
npm run redundancy:validate
npm run redundancy:cleanup-duplicates
```

## Usage Examples

### Generate Comprehensive Report
```bash
npm run redundancy:check -- --output=reports/redundancy-check.md
```

### Clean Specific Types
```bash
npm run redundancy:check -- --type=vector --cleanup --output=reports/cleanup-report.md
```

### Custom Commands
```typescript
import { RedundancyChecker } from './src/redundancy-checker';
import { PersistentVectorStore, PersistentRelationshipStore } from './src/storage';

async function myRedundancyCheck() {
  const vectorStore = new PersistentVectorStore();
  const relationshipStore = new PersistentRelationshipStore();
  
  await vectorStore.initialize();
  await relationshipStore.initialize();
  
  const checker = new RedundancyChecker(vectorStore, relationshipStore);
  
  // Run specific checks
  const vectorReport = await checker.checkVectorRedundancy();
  const relationshipReport = await checker.checkRelationshipRedundancy();
  
  // Cleanup if needed
  if (vectorReport.duplicateCount > 0) {
    await checker.cleanupRedundancy(vectorReport);
  }
  
  // Generate full report
  const report = await checker.generateReport();
  console.log(report);
}
```

## What Gets Checked

### 1. Vector Redundancy
- Duplicate embeddings
- Duplicate vector content
- Space calculation (float32 bytes)

### 2. Relationship Redundancy
- Duplicate relationships (same source → target → type → metadata)
- Stale relationships
- Circular references

### 3. Process Redundancy
- Multiple running embedding processes
- Duplicate worker processes
- Memory waste estimation

### 4. Code Base Redundancy
- Duplicate function implementations
- Repeated configuration
- Code duplication within threshold

## Expected Outputs

```
🔍 Cortex Redundancy Checker
============================

# Vector Redundancy Report
- Total Items: 1582
- Duplicates: 47
- Space Wasted: 1.8MB

# Relationship Redundancy Report  
- Total Items: 892
- Duplicates: 12
- Space Wasted: 12KB

✅ Vector redundancy cleanup completed: removed 47 duplicates
```

## Periodic Monitoring

Add to your cron or scheduled tasks:

```bash
# Daily check and report
0 2 * * * cd /path/to/cortexyoung && npm run redundancy:check -- --output=reports/daily-$(date +%Y%m%d).md

# Weekly cleanup
0 3 * * 0 npm run redundancy:cleanup:vector && npm run redundancy:cleanup:relationship
```

## Configuration

The system automatically handles:
- Vector dimension calculation (uses actual embedding size)
- Hash-based deduplication (SHA256 for vectors, MD5 for relationships)
- Safe cleanup (keeps first occurrence, removes duplicates)
- Space estimation and reporting
- Progress logging

## Files Created

- `reports/redundancy-*.md` - Detailed reports
- `src/redundancy-checker.ts` - Main checker class
- `src/cli/redundancy-check.ts` - Command-line interface
- `src/validation/relationship-redundancy.ts` - Relationship-specific validation
- `src/validation/cleanup-duplicates.ts` - Cleanup script

## Troubleshooting

### Permission Issues
```bash
sudo chown -R $(whoami) ./reports/
chmod -R 755 ./reports/
```

### Memory Issues with Large Datasets
```bash
node --max-old-space-size=4096 ./node_modules/.bin/ts-node -r ts-node/register src/cli/redundancy-check.ts
```

### Stale Locks
```bash
# Force cleanup of temporary files
rm -rf ./tmp/redundancy-*
```