// This script tests the CloudflareAIEmbedder class directly.

const { CloudflareAIEmbedder } = require('./dist/cloudflare-ai-embedder.js');

async function testCloudflareClass() {
  console.log('--- Testing CloudflareAIEmbedder Class ---');
  
  const embedder = new CloudflareAIEmbedder();

  try {
    // Test 1: Single text embedding
    console.log('\n--- Test 1: embedSingle() ---');
    const singleText = 'This is a test of the class.';
    const singleEmbedding = await embedder.embedSingle(singleText);
    console.log('Successfully received embedding for single text.');
    console.log(`  - Vector dimensions: ${singleEmbedding.length}`);
    console.log(`  - Sample values: [${singleEmbedding.slice(0, 5).join(', ')}, ...]`);
    if (singleEmbedding.length !== 384) {
        throw new Error(`Expected vector dimension 384, but got ${singleEmbedding.length}`);
    }

    // Test 2: Batch text embedding
    console.log('\n--- Test 2: embed() with a batch ---');
    const batchTexts = ['First item in the batch.', 'Second item for testing.'];
    const batchEmbeddings = await embedder.embed(batchTexts);
    console.log('Successfully received embeddings for batch text.');
    console.log(`  - Number of vectors: ${batchEmbeddings.length}`);
    console.log(`  - Vector dimensions: ${batchEmbeddings[0].length}`);
    console.log(`  - Sample values (first vector): [${batchEmbeddings[0].slice(0, 5).join(', ')}, ...]`);
    if (batchEmbeddings.length !== 2 || batchEmbeddings[0].length !== 384) {
        throw new Error(`Batch test failed. Expected 2 vectors of 384 dimensions.`);
    }

    console.log('\nAll class tests passed!');

  } catch (error) {
    console.error('\nError during class testing:', error.message);
  }
}

testCloudflareClass();