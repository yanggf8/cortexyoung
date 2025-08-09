export default {
  async fetch(request, env, ctx) {
    if (request.method !== 'POST') {
      return new Response('Expected POST request', { status: 405 });
    }

    const { text, texts } = await request.json();

    if (!text && !texts) {
      return new Response('Missing "text" or "texts" in request body', { status: 400 });
    }

    const model = '@cf/baai/bge-small-en-v1.5';
    let embeddings;

    try {
      if (texts && texts.length > 0) {
        // Batch processing
        const response = await env.AI.run(model, { text: texts });
        embeddings = response.data;
      } else if (text) {
        // Single text processing
        const response = await env.AI.run(model, { text: [text] });
        embeddings = response.data[0]; // Return the single embedding directly
      } else {
        return new Response('Request body must contain either a "text" string or a "texts" array.', { status: 400 });
      }

      return new Response(JSON.stringify({ embeddings }), {
        headers: { 'Content-Type': 'application/json' },
      });

    } catch (e) {
      return new Response(e.message, { status: 500 });
    }
  },
};