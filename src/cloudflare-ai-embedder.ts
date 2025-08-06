const WORKER_URL = 'https://cortex-embedder.yanggf.workers.dev';
const BATCH_SIZE = 100; // As per Cloudflare's documented limit

interface EmbeddingResponse {
  embeddings: number[][];
}

interface SingleEmbeddingResponse {
    embeddings: number[];
}

export class CloudflareAIEmbedder {
  public readonly dimensions = 384; // For bge-small-en-v1.5

  async embed(texts: string[]): Promise<number[][]> {
    const allEmbeddings: number[][] = [];

    for (let i = 0; i < texts.length; i += BATCH_SIZE) {
      const batch = texts.slice(i, i + BATCH_SIZE);
      
      try {
        const response = await fetch(WORKER_URL, {
          method: 'POST',
          headers: { 'Content-Type': 'application/json' },
          body: JSON.stringify({ texts: batch }),
        });

        if (!response.ok) {
          const errorText = await response.text();
          throw new Error(`Cloudflare AI request failed with status ${response.status}: ${errorText}`);
        }

        const data = (await response.json()) as EmbeddingResponse;
        allEmbeddings.push(...data.embeddings);

      } catch (error) {
        console.error('Error embedding batch with Cloudflare AI:', error);
        // Depending on desired behavior, you might want to re-throw or handle this differently
        throw error;
      }
    }

    return allEmbeddings;
  }

  async embedSingle(text: string): Promise<number[]> {
    try {
      const response = await fetch(WORKER_URL, {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ text }),
      });

      if (!response.ok) {
        const errorText = await response.text();
        throw new Error(`Cloudflare AI request failed with status ${response.status}: ${errorText}`);
      }

      const data = (await response.json()) as SingleEmbeddingResponse;
      return data.embeddings;

    } catch (error) {
      console.error('Error embedding single text with Cloudflare AI:', error);
      throw error;
    }
  }
}