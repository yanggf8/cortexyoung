# Cortex V5.0 Worker — Deployment Guide

## Prerequisites

- Cloudflare account with Workers enabled
- Turso account (free tier: https://turso.tech)
- Wrangler CLI (installed via `npm install` in this directory)

## Setup Steps

### 1. Create Turso Database

```bash
# Install turso CLI
curl -sSfL https://get.tur.so/install.sh | bash

# Authenticate
turso auth login

# Create database
turso db create cortex-v5

# Get connection URL
turso db show cortex-v5 --url
# e.g., libsql://cortex-v5-xxx.turso.io

# Create auth token
turso db tokens create cortex-v5
```

### 2. Set Worker Secrets

```bash
npx wrangler secret put TURSO_URL
# Paste: libsql://cortex-v5-xxx.turso.io

npx wrangler secret put TURSO_AUTH_TOKEN
# Paste: token from turso db tokens create

npx wrangler secret put SETUP_TOKEN
# Paste: a random string for initial setup (generate with: openssl rand -hex 32)
```

### 3. Deploy

```bash
npx wrangler deploy
```

Note the Worker URL (e.g., `https://cortex-v5.<account>.workers.dev`).

### 4. Initialize Schema

```bash
# Use the SETUP_TOKEN you set in step 2
export WORKER_URL="https://cortex-v5.<account>.workers.dev"
export SETUP_TOKEN="your-setup-token"

curl -X POST "$WORKER_URL/admin/init-schema" \
  -H "Authorization: Bearer $SETUP_TOKEN" \
  -H "Content-Type: application/json"
```

### 5. Register API Key

```bash
# Generate an API key
export API_KEY=$(openssl rand -hex 32)

curl -X POST "$WORKER_URL/admin/register-key" \
  -H "Authorization: Bearer $SETUP_TOKEN" \
  -H "Content-Type: application/json" \
  -d "{\"api_key\": \"$API_KEY\"}"

# Save this API key — it goes into ~/.cortex/config.json
echo "Your API key: $API_KEY"
```

## Testing with curl

Set environment variables:

```bash
export WORKER_URL="https://cortex-v5.<account>.workers.dev"
export API_KEY="your-api-key"
```

### Health Check

```bash
curl "$WORKER_URL/health" \
  -H "Authorization: Bearer $API_KEY"
```

Expected:
```json
{"status":"healthy","turso":true}
```

### Index Batch

```bash
curl -X POST "$WORKER_URL/index/batch" \
  -H "Authorization: Bearer $API_KEY" \
  -H "Content-Type: application/json" \
  -d '{
    "project_id": "test-project",
    "project_name": "Test Project",
    "project_path": "/tmp/test",
    "chunks": [
      {
        "chunk_id": "test-project:src/index.ts:1",
        "project_id": "test-project",
        "file_path": "src/index.ts",
        "symbol_name": "main",
        "chunk_type": "function",
        "start_line": 1,
        "end_line": 10,
        "content": "function main() { console.log(\"hello\"); }",
        "content_hash": "abc123",
        "language": "typescript",
        "embedding": [0.1, 0.2, 0.3, 0.4, 0.5, 0.6, 0.7, 0.8, 0.9, 1.0, 0.1, 0.2, 0.3, 0.4, 0.5, 0.6, 0.7, 0.8, 0.9, 1.0, 0.1, 0.2, 0.3, 0.4, 0.5, 0.6, 0.7, 0.8, 0.9, 1.0, 0.1, 0.2, 0.3, 0.4, 0.5, 0.6, 0.7, 0.8, 0.9, 1.0, 0.1, 0.2, 0.3, 0.4, 0.5, 0.6, 0.7, 0.8, 0.9, 1.0, 0.1, 0.2, 0.3, 0.4, 0.5, 0.6, 0.7, 0.8, 0.9, 1.0, 0.1, 0.2, 0.3, 0.4, 0.5, 0.6, 0.7, 0.8, 0.9, 1.0, 0.1, 0.2, 0.3, 0.4, 0.5, 0.6, 0.7, 0.8, 0.9, 1.0, 0.1, 0.2, 0.3, 0.4, 0.5, 0.6, 0.7, 0.8, 0.9, 1.0, 0.1, 0.2, 0.3, 0.4, 0.5, 0.6, 0.7, 0.8, 0.9, 1.0, 0.1, 0.2, 0.3, 0.4, 0.5, 0.6, 0.7, 0.8, 0.9, 1.0, 0.1, 0.2, 0.3, 0.4, 0.5, 0.6, 0.7, 0.8, 0.9, 1.0, 0.1, 0.2, 0.3, 0.4, 0.5, 0.6, 0.7, 0.8, 0.9, 1.0, 0.1, 0.2, 0.3, 0.4, 0.5, 0.6, 0.7, 0.8, 0.9, 1.0, 0.1, 0.2, 0.3, 0.4, 0.5, 0.6, 0.7, 0.8, 0.9, 1.0, 0.1, 0.2, 0.3, 0.4, 0.5, 0.6, 0.7, 0.8, 0.9, 1.0, 0.1, 0.2, 0.3, 0.4, 0.5, 0.6, 0.7, 0.8, 0.9, 1.0, 0.1, 0.2, 0.3, 0.4, 0.5, 0.6, 0.7, 0.8, 0.9, 1.0, 0.1, 0.2, 0.3, 0.4, 0.5, 0.6, 0.7, 0.8, 0.9, 1.0, 0.1, 0.2, 0.3, 0.4, 0.5, 0.6, 0.7, 0.8, 0.9, 1.0, 0.1, 0.2, 0.3, 0.4, 0.5, 0.6, 0.7, 0.8, 0.9, 1.0, 0.1, 0.2, 0.3, 0.4, 0.5, 0.6, 0.7, 0.8, 0.9, 1.0, 0.1, 0.2, 0.3, 0.4, 0.5, 0.6, 0.7, 0.8, 0.9, 1.0, 0.1, 0.2, 0.3, 0.4, 0.5, 0.6, 0.7, 0.8, 0.9, 1.0, 0.1, 0.2, 0.3, 0.4, 0.5, 0.6, 0.7, 0.8, 0.9, 1.0, 0.1, 0.2, 0.3, 0.4, 0.5, 0.6, 0.7, 0.8, 0.9, 1.0, 0.1, 0.2, 0.3, 0.4, 0.5, 0.6, 0.7, 0.8, 0.9, 1.0, 0.1, 0.2, 0.3, 0.4, 0.5, 0.6, 0.7, 0.8, 0.9, 1.0, 0.1, 0.2, 0.3, 0.4, 0.5, 0.6, 0.7, 0.8, 0.9, 1.0, 0.1, 0.2, 0.3, 0.4, 0.5, 0.6, 0.7, 0.8, 0.9, 1.0, 0.1, 0.2, 0.3, 0.4, 0.5, 0.6, 0.7, 0.8, 0.9, 1.0, 0.1, 0.2, 0.3, 0.4, 0.5, 0.6, 0.7, 0.8, 0.9, 1.0, 0.1, 0.2, 0.3, 0.4, 0.5, 0.6, 0.7, 0.8, 0.9, 1.0, 0.1, 0.2, 0.3, 0.4, 0.5, 0.6, 0.7, 0.8, 0.9, 1.0, 0.1, 0.2, 0.3, 0.4, 0.5, 0.6, 0.7, 0.8, 0.9, 1.0, 0.1, 0.2, 0.3, 0.4, 0.5, 0.6, 0.7, 0.8, 0.9, 1.0, 0.1, 0.2, 0.3, 0.4, 0.5, 0.6, 0.7, 0.8, 0.9, 1.0, 0.1, 0.2, 0.3, 0.4, 0.5, 0.6, 0.7, 0.8, 0.9, 1.0, 0.1, 0.2, 0.3, 0.4, 0.5, 0.6, 0.7, 0.8, 0.9, 1.0, 0.1, 0.2, 0.3, 0.4]
      }
    ],
    "relationships": []
  }'
```

Expected:
```json
{"indexed":1}
```

### Search (requires real 384-dim embedding vector)

```bash
# The vector must be a real embedding from @xenova/transformers
# For testing, you can use the same dummy vector from above
curl -X POST "$WORKER_URL/search" \
  -H "Authorization: Bearer $API_KEY" \
  -H "Content-Type: application/json" \
  -d '{
    "vector": [0.1, 0.2, ...384 values...],
    "project_id": "test-project",
    "top_k": 5
  }'
```

### Keyword Search

```bash
curl -X POST "$WORKER_URL/search/keyword" \
  -H "Authorization: Bearer $API_KEY" \
  -H "Content-Type: application/json" \
  -d '{
    "query": "main function",
    "project_id": "test-project"
  }'
```

### Relationships

```bash
curl -X POST "$WORKER_URL/relationships" \
  -H "Authorization: Bearer $API_KEY" \
  -H "Content-Type: application/json" \
  -d '{
    "symbol": "main",
    "project_id": "test-project",
    "depth": 2,
    "rel_types": ["calls", "called_by"]
  }'
```

### Project Status

```bash
curl "$WORKER_URL/projects/test-project/status" \
  -H "Authorization: Bearer $API_KEY"
```

### List Projects

```bash
curl "$WORKER_URL/projects" \
  -H "Authorization: Bearer $API_KEY"
```

### Delete File

```bash
curl -X POST "$WORKER_URL/index/delete-file" \
  -H "Authorization: Bearer $API_KEY" \
  -H "Content-Type: application/json" \
  -d '{
    "project_id": "test-project",
    "file_path": "src/index.ts"
  }'
```

### Delete Project

```bash
curl -X DELETE "$WORKER_URL/projects/test-project" \
  -H "Authorization: Bearer $API_KEY"
```

## Key Rotation

```bash
export NEW_KEY=$(openssl rand -hex 32)
curl -X POST "$WORKER_URL/admin/rotate-key" \
  -H "Authorization: Bearer $SETUP_TOKEN" \
  -H "Content-Type: application/json" \
  -d "{\"old_key\": \"$API_KEY\", \"new_key\": \"$NEW_KEY\"}"
```
