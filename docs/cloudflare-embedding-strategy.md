# Cloudflare Workers AI Embedding Strategy

This document outlines the strategy for leveraging Cloudflare Workers AI to accelerate and scale text embedding generation for the Cortex project.

## Overview

The core concept is to offload the computationally intensive task of embedding from the local application to Cloudflare's global, serverless network. Instead of using a local Python process or a Node.js worker pool, the application will make API calls to a dedicated Cloudflare Worker.

This approach provides two primary benefits:
1.  **Massive Parallelism**: Cloudflare's infrastructure can handle thousands of simultaneous requests, dramatically increasing embedding throughput.
2.  **Reduced Local Resource Load**: Frees up the local machine's CPU to focus on core application logic like indexing, search, and data coordination.

## Implementation

A Cloudflare Worker was created and deployed to handle embedding requests. 

- **Worker Script**: `cloudflare-worker.js`
- **Deployment Tool**: Cloudflare Wrangler CLI (`wrangler`)
- **Configuration**: `wrangler.toml`
- **Deployment URL**: `https://cortex-embedder.yanggf.workers.dev`

The worker is configured to use the `@cf/baai/bge-small-en-v1.5` model, which is compatible with the existing local embedding setup.

## Throughput Analysis

The performance (throughput) of this strategy is determined by Cloudflare's service limits.

- **Model**: `@cf/baai/bge-small-en-v1.5`
- **Rate Limit**: 3,000 requests per minute (or 50 requests per second).
- **Batch Size**: 100 texts (chunks) per request.

### Theoretical Maximum Throughput

The maximum possible throughput is calculated as:

`50 requests/second * 100 chunks/request = 5,000 chunks per second`

### Practical Throughput (Free Plan)

Accounts on the Cloudflare Free plan are subject to a lower burst rate limit of 1,000 requests per minute.

` (1,000 requests/minute / 60 seconds/minute) * 100 chunks/request ≈ 1,667 chunks per second`

'''This represents a significant performance increase over a single-machine setup and provides a clear path for scaling with a paid plan if needed.

## Usage

The embedding engine can be selected at runtime using the `EMBEDDER_TYPE` environment variable.

- **Local Embedding (Default)**: Uses the `ProcessPoolEmbedder` for local, CPU-based embedding.
  ```bash
  npm start
  ```

- **Cloudflare Embedding**: Uses the `CloudflareAIEmbedder` to offload embedding to the deployed worker.
  ```bash
  npm run start:cloudflare
  ```
  Alternatively, you can set the environment variable directly:
  ```bash
  EMBEDDER_TYPE=cloudflare npm start
  ```'''
