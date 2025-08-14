#!/usr/bin/env node

import { RedundantRelationshipValidator } from './relationship-redundancy';

async function cleanupDuplicates() {
  console.log('🧹 Starting duplicate relationship cleanup...');
  
  try {
    const validator = new RedundantRelationshipValidator(
      new (await import('../persistent-relationship-store')).PersistentRelationshipStore(process.cwd())
    );
    
    const result = await validator.cleanupDuplicates();
    
    console.log(`\n✅ Cleanup completed successfully:`);
    console.log(`- Removed ${result.cleaned} duplicate relationships`);
    console.log(`- Saved approximately ${result.savedSpace.toLocaleString()} bytes`);
    
  } catch (error) {
    console.error('❌ Cleanup failed:', error);
    process.exit(1);
  }
}

if (require.main === module) {
  cleanupDuplicates().catch(console.error);
}