import assert from 'node:assert/strict';
import {
  resolvePreferredProjectId,
  resolveRelationshipsForTest,
  resolveImportTargetsForTest,
  getFileKeysForTest,
  createImportResolverForTest,
} from '../dist/index.js';

function testProjectSelection() {
  assert.equal(
    resolvePreferredProjectId('cwd-project', true, 'default-project'),
    'cwd-project',
    'cwd project should win when it exists'
  );

  assert.equal(
    resolvePreferredProjectId('cwd-project', false, 'default-project'),
    'default-project',
    'default project should be used when cwd project is not indexed'
  );

  assert.equal(
    resolvePreferredProjectId('cwd-project', false, ''),
    'cwd-project',
    'cwd hash should remain the fallback when no default project exists'
  );
}

function testRelativeImportResolution() {
  const fileIndex = new Map();
  for (const key of getFileKeysForTest('src/utils/math.ts')) {
    fileIndex.set(key, 'chunk-math');
  }

  assert.deepEqual(
    resolveImportTargetsForTest('./utils/math', 'src/app.ts', fileIndex),
    ['chunk-math'],
    'relative imports should resolve to local chunk ids'
  );
}

function testBareImportDoesNotResolveToLocalFile() {
  const fileIndex = new Map();
  for (const key of getFileKeysForTest('src/promises.ts')) {
    fileIndex.set(key, 'chunk-promises');
  }

  assert.deepEqual(
    resolveImportTargetsForTest('fs/promises', 'src/app.ts', fileIndex),
    [],
    'bare package specifiers must not create false local edges'
  );
}

function testAliasImportResolvesToLocalFile() {
  const fileIndex = new Map();
  for (const key of getFileKeysForTest('src/utils/math.ts')) {
    fileIndex.set(key, 'chunk-math');
  }

  const importResolver = createImportResolverForTest('.', undefined, [
    {
      findPrefix: '@/',
      findSuffix: '',
      replacements: [{ prefix: 'src/', suffix: '' }],
    },
  ]);

  assert.deepEqual(
    resolveImportTargetsForTest('@/utils/math', 'src/app.ts', fileIndex, importResolver),
    ['chunk-math'],
    'configured alias imports should resolve to local chunk ids'
  );
}

function testRelationshipResolution() {
  const symbolIndex = new Map([
    ['helper', new Set(['chunk-helper'])],
    ['exportedThing', new Set(['chunk-export'])],
  ]);
  const fileIndex = new Map();
  for (const key of getFileKeysForTest('src/lib/module.ts')) {
    fileIndex.set(key, 'chunk-module');
  }

  const rels = resolveRelationshipsForTest(
    [
      { source_chunk_id: 'chunk-source', source_file_path: 'src/app.ts', target_ref: 'helper', rel_type: 'calls' },
      { source_chunk_id: 'chunk-source', source_file_path: 'src/app.ts', target_ref: './lib/module', rel_type: 'imports' },
      { source_chunk_id: 'chunk-source', source_file_path: 'src/app.ts', target_ref: 'exportedThing', rel_type: 'exports' },
    ],
    symbolIndex,
    fileIndex
  );

  assert.deepEqual(rels, [
    { source_chunk_id: 'chunk-source', target_chunk_id: 'chunk-helper', rel_type: 'calls' },
    { source_chunk_id: 'chunk-source', target_chunk_id: 'chunk-module', rel_type: 'imports' },
    { source_chunk_id: 'chunk-source', target_chunk_id: 'chunk-export', rel_type: 'exports' },
  ]);
}

testProjectSelection();
testRelativeImportResolution();
testBareImportDoesNotResolveToLocalFile();
testAliasImportResolvesToLocalFile();
testRelationshipResolution();

console.log('phase2 smoke tests passed');
