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

// Batch size for the transformers pipeline: it runs the whole group through
// one forward pass, so this trades peak RSS (~200MB model + batch tensors)
// against per-call overhead. 32 keeps memory flat on large files.
const EMBED_BATCH_SIZE = 32;

export async function embedBatch(texts: string[]): Promise<number[][]> {
  if (!extractor) await loadModel();
  if (texts.length === 0) return [];
  const embeddings: number[][] = [];
  for (let i = 0; i < texts.length; i += EMBED_BATCH_SIZE) {
    const group = texts.slice(i, i + EMBED_BATCH_SIZE);
    const result = await extractor(group, { pooling: 'cls', normalize: true });
    // Output tensor is [batch, dim] flattened; split rows back apart.
    const dim = result.dims[result.dims.length - 1] as number;
    for (let j = 0; j < group.length; j++) {
      embeddings.push(Array.from(result.data.slice(j * dim, (j + 1) * dim) as Float32Array));
    }
  }
  return embeddings;
}
