// This is a test script to validate your Cloudflare Worker for embedding.
// Before running, make sure you have deployed the worker and replaced the URL.

const WORKER_URL = 'https://cortex-embedder.yanggf.workers.dev'; // <-- IMPORTANT: Replace with your actual worker URL

async function testEmbeddingWorker() {
  if (WORKER_URL === 'YOUR_WORKER_URL') {
    console.error('Please replace YOUR_WORKER_URL in this script with the actual URL of your deployed Cloudflare Worker.');
    return;
  }

  console.log(`Testing worker at: ${WORKER_URL}`);

  try {
    // Test 1: Single text embedding
    console.log('\n--- Test 1: Single Text Embedding ---');
    const singleText = 'Hello, world!';
    let response = await fetch(WORKER_URL, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ text: singleText }),
    });

    if (!response.ok) {
      throw new Error(`Single text test failed with status ${response.status}: ${await response.text()}`);
    }

    let data = await response.json();
    console.log('Successfully received embedding for single text.');
    console.log(`  - Vector dimensions: ${data.embeddings.length}`);
    console.log(`  - Sample values: [${data.embeddings.slice(0, 5).join(', ')}, ...]`);

    // Test 2: Batch text embedding
    console.log('\n--- Test 2: Batch Text Embedding ---');
    const batchTexts = ['This is the first sentence.', 'Here is a second one.', 'And a third for good measure.'];
    response = await fetch(WORKER_URL, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ texts: batchTexts }),
    });

    if (!response.ok) {
      throw new Error(`Batch text test failed with status ${response.status}: ${await response.text()}`);
    }

    data = await response.json();
    console.log('Successfully received embeddings for batch text.');
    console.log(`  - Number of vectors: ${data.embeddings.length}`);
    console.log(`  - Vector dimensions: ${data.embeddings[0].length}`);
    console.log(`  - Sample values (first vector): [${data.embeddings[0].slice(0, 5).join(', ')}, ...]`);

  } catch (error) {
    console.error('\nError during testing:', error.message);
  }
}

testEmbeddingWorker();