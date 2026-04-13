import assert from 'node:assert/strict';
import {
  resolvePreferredProjectId,
  resolveRelationshipsForTest,
  resolveImportTargetsForTest,
  getFileKeysForTest,
  createImportResolverForTest,
  parseQueryFilters,
  hasFilters,
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
    { source_chunk_id: 'chunk-source', target_chunk_id: 'chunk-helper', rel_type: 'calls', confidence: 'INFERRED', confidence_score: 0.5599999999999999, confidence_reasoning: 'single symbol-name match (0.70) × INFERRED (0.8) = 0.56' },
    { source_chunk_id: 'chunk-source', target_chunk_id: 'chunk-module', rel_type: 'imports', confidence: 'EXTRACTED', confidence_score: 1, confidence_reasoning: 'file-index resolution (1.00) × EXTRACTED (1.0) = 1.00' },
    { source_chunk_id: 'chunk-source', target_chunk_id: 'chunk-export', rel_type: 'exports', confidence: 'EXTRACTED', confidence_score: 0.7, confidence_reasoning: 'single symbol-name match (0.70) × EXTRACTED (1.0) = 0.70' },
  ]);
}

function testParseQueryFiltersBasic() {
  const { textQuery, filters } = parseQueryFilters('useEffect kind:function lang:ts');
  assert.equal(textQuery, 'useEffect');
  assert.equal(filters.kind, 'function');
  assert.equal(filters.language, 'typescript');
  assert.equal(filters.symbolGlob, undefined);
  assert.equal(filters.fileGlob, undefined);
  assert.equal(hasFilters(filters), true);
}

function testParseQueryFiltersGlob() {
  const { textQuery, filters } = parseQueryFilters('name:parse* file:src/auth/**');
  assert.equal(textQuery, '');
  assert.equal(filters.symbolGlob, 'parse%');
  assert.equal(filters.fileGlob, 'src/auth/%');
}

function testParseQueryFiltersAliases() {
  // `method` aliases to `function`; `interface` aliases to `config`; py → python.
  const { filters } = parseQueryFilters('foo kind:method lang:py');
  assert.equal(filters.kind, 'function');
  assert.equal(filters.language, 'python');

  const { filters: configFilters } = parseQueryFilters('foo kind:interface');
  assert.equal(configFilters.kind, 'config');
}

function testParseQueryFiltersEscapesLikeWildcards() {
  // SQL LIKE wildcards in raw input must be escaped, not pass through.
  const { filters } = parseQueryFilters('name:foo_bar%baz');
  assert.equal(filters.symbolGlob, 'foo\\_bar\\%baz');
}

function testParseQueryFiltersNoFilters() {
  const { textQuery, filters } = parseQueryFilters('plain query');
  assert.equal(textQuery, 'plain query');
  assert.equal(hasFilters(filters), false);
}

testProjectSelection();
testRelativeImportResolution();
testBareImportDoesNotResolveToLocalFile();
testAliasImportResolvesToLocalFile();
testRelationshipResolution();
testParseQueryFiltersBasic();
testParseQueryFiltersGlob();
testParseQueryFiltersAliases();
testParseQueryFiltersEscapesLikeWildcards();
testParseQueryFiltersNoFilters();

console.log('phase2 smoke tests passed');
