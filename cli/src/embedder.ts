import { pipeline } from '@xenova/transformers';

let extractor: any = null;

export async function loadModel(): Promise<void> {
  if (extractor) return;
  extractor = await pipeline('feature-extraction', 'Xenova/bge-small-en-v1.5');
}

export async function embed(text: string): Promise<number[]> {
  if (!extractor) await loadModel();
  const result = await extractor(text, { pooling: 'cls', normalize: true });
  return Array.from(result.data as Float32Array);
}

export async function embedBatch(texts: string[]): Promise<number[][]> {
  if (!extractor) await loadModel();
  const embeddings: number[][] = [];
  for (const text of texts) {
    const result = await extractor(text, { pooling: 'cls', normalize: true });
    embeddings.push(Array.from(result.data as Float32Array));
  }
  return embeddings;
}
