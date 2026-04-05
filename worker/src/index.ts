// Cortex V5.0 Cloudflare Worker — main entry point
import type { Env, BatchIndexRequest, SearchRequest, KeywordSearchRequest, RelationshipRequest, DeleteFileRequest } from './types';
import { authenticate, validateSetupToken, registerKey, revokeKey } from './auth';
import { getTursoClient, ensureForeignKeys, applySchema, upsertChunks, upsertRelationships, deleteChunksByFile, deleteProjectChunks, vectorSearch, keywordSearch as tursoKeywordSearch, traverseRelationships, getProjectStatus, listProjects, upsertProject } from './turso-client';

const DEFAULT_TOP_K = 15;
const MAX_TOP_K = 100;
const MAX_RELATIONSHIP_DEPTH = 3;
const MAX_BATCH_SIZE = 100;
const CHUNK_ID_REGEX = /^[^:]+:[^:]+:\d+$/;

function jsonResponse(data: any, status = 200): Response {
  return new Response(JSON.stringify(data), {
    status,
    headers: {
      'Content-Type': 'application/json',
      'Access-Control-Allow-Origin': '*',
    },
  });
}

function errorResponse(message: string, status = 400): Response {
  return jsonResponse({ error: message }, status);
}

export default {
  async fetch(request: Request, env: Env): Promise<Response> {
    const url = new URL(request.url);
    const path = url.pathname;

    // CORS preflight
    if (request.method === 'OPTIONS') {
      return new Response(null, {
        headers: {
          'Access-Control-Allow-Origin': '*',
          'Access-Control-Allow-Methods': 'GET, POST, DELETE, OPTIONS',
          'Access-Control-Allow-Headers': 'Authorization, Content-Type',
        },
      });
    }

    // Route: Admin — register API key (uses setup token, not API key)
    if (path === '/admin/register-key' && request.method === 'POST') {
      return handleRegisterKey(request, env);
    }

    // Route: Admin — rotate key
    if (path === '/admin/rotate-key' && request.method === 'POST') {
      return handleRotateKey(request, env);
    }

    // Route: Admin — init schema
    if (path === '/admin/init-schema' && request.method === 'POST') {
      return handleInitSchema(request, env);
    }

    // All other routes require API key auth
    const auth = await authenticate(request, env);
    if (!auth.authenticated) {
      return errorResponse(auth.error!, 401);
    }

    try {
      // Ensure foreign keys are enabled for this connection
      await ensureForeignKeys(env);

      // Health check
      if (path === '/health' && request.method === 'GET') {
        return handleHealth(env);
      }

      // Index batch
      if (path === '/index/batch' && request.method === 'POST') {
        return handleIndexBatch(request, env);
      }

      // Delete file
      if (path === '/index/delete-file' && request.method === 'POST') {
        return handleDeleteFile(request, env);
      }

      // Search (vector)
      if (path === '/search' && request.method === 'POST') {
        return handleSearch(request, env);
      }

      // Search (keyword FTS5)
      if (path === '/search/keyword' && request.method === 'POST') {
        return handleKeywordSearch(request, env);
      }

      // Relationships
      if (path === '/relationships' && request.method === 'POST') {
        return handleRelationships(request, env);
      }

      // List projects
      if (path === '/projects' && request.method === 'GET') {
        return handleListProjects(env);
      }

      // Project status
      const projectMatch = path.match(/^\/projects\/([^/]+)\/status$/);
      if (projectMatch && request.method === 'GET') {
        return handleProjectStatus(projectMatch[1], env);
      }

      // Delete project
      const projectDeleteMatch = path.match(/^\/projects\/([^/]+)$/);
      if (projectDeleteMatch && request.method === 'DELETE') {
        return handleDeleteProject(projectDeleteMatch[1], env);
      }

      return errorResponse('Not found', 404);
    } catch (err: any) {
      console.error(`Error on ${path}:`, err);
      return errorResponse(err.message || 'Internal server error', 500);
    }
  },
};

// --- Admin handlers ---

async function handleRegisterKey(request: Request, env: Env): Promise<Response> {
  if (!validateSetupToken(request, env)) {
    return errorResponse('Invalid setup token', 401);
  }
  const body = await request.json() as { api_key?: string };
  if (!body.api_key) {
    return errorResponse('Missing api_key');
  }
  await registerKey(body.api_key, env);
  return jsonResponse({ registered: true });
}

async function handleRotateKey(request: Request, env: Env): Promise<Response> {
  if (!validateSetupToken(request, env)) {
    return errorResponse('Invalid setup token', 401);
  }
  const body = await request.json() as { old_key?: string; new_key?: string };
  if (!body.old_key || !body.new_key) {
    return errorResponse('Missing old_key or new_key');
  }
  await revokeKey(body.old_key, env);
  await registerKey(body.new_key, env);
  return jsonResponse({ rotated: true });
}

async function handleInitSchema(request: Request, env: Env): Promise<Response> {
  if (!validateSetupToken(request, env)) {
    return errorResponse('Invalid setup token', 401);
  }
  await applySchema(env);
  return jsonResponse({ initialized: true });
}

// --- Health ---

async function handleHealth(env: Env): Promise<Response> {
  let tursoOk = false;
  try {
    const db = getTursoClient(env);
    await db.execute('SELECT 1');
    tursoOk = true;
  } catch { /* turso down */ }

  return jsonResponse({
    status: tursoOk ? 'healthy' : 'degraded',
    turso: tursoOk,
  });
}

// --- Index batch ---

async function handleIndexBatch(request: Request, env: Env): Promise<Response> {
  const body = await request.json() as BatchIndexRequest;

  if (!body.project_id || !body.chunks || !Array.isArray(body.chunks)) {
    return errorResponse('Missing project_id or chunks array');
  }

  if (body.chunks.length > MAX_BATCH_SIZE) {
    return errorResponse(`Batch size exceeds ${MAX_BATCH_SIZE}. Split into multiple requests.`);
  }

  // Validate chunk_id format
  for (const chunk of body.chunks) {
    if (!CHUNK_ID_REGEX.test(chunk.chunk_id)) {
      return errorResponse(`Invalid chunk_id format: ${chunk.chunk_id}. Expected project_id:file_path:start_line`);
    }
    if (!chunk.embedding || chunk.embedding.length !== 384) {
      return errorResponse(`Invalid embedding for chunk ${chunk.chunk_id}: expected 384 dimensions`);
    }
  }

  // Upsert chunks to Turso (content + metadata + embeddings)
  await upsertChunks(env, body.chunks.map(c => ({
    chunk_id: c.chunk_id,
    project_id: c.project_id,
    file_path: c.file_path,
    symbol_name: c.symbol_name,
    chunk_type: c.chunk_type,
    start_line: c.start_line,
    end_line: c.end_line,
    content: c.content,
    content_hash: c.content_hash,
    language: c.language,
    embedding: c.embedding,
  })));

  // Upsert relationships to Turso
  if (body.relationships && body.relationships.length > 0) {
    await upsertRelationships(env, body.relationships);
  }

  // Update project record
  await upsertProject(env, body.project_id, body.project_name ?? body.project_id, body.project_path);

  return jsonResponse({ indexed: body.chunks.length });
}

// --- Delete file ---

async function handleDeleteFile(request: Request, env: Env): Promise<Response> {
  const body = await request.json() as DeleteFileRequest;

  if (!body.project_id || !body.file_path) {
    return errorResponse('Missing project_id or file_path');
  }

  const chunkIds = await deleteChunksByFile(env, body.project_id, body.file_path);

  return jsonResponse({ deleted: chunkIds.length, chunk_ids: chunkIds });
}

// --- Search (vector) ---

async function handleSearch(request: Request, env: Env): Promise<Response> {
  const body = await request.json() as SearchRequest;

  if (!body.vector || !body.project_id) {
    return errorResponse('Missing vector or project_id');
  }

  if (body.vector.length !== 384) {
    return errorResponse('Invalid vector dimensions: expected 384');
  }

  const topK = Math.min(body.top_k ?? DEFAULT_TOP_K, MAX_TOP_K);
  const offset = body.offset ?? 0;

  const result = await vectorSearch(env, body.vector, body.project_id, topK, offset);

  return jsonResponse({
    chunks: result.chunks,
    total_matches: result.total_matches,
    has_more: result.has_more,
  });
}

// --- Search (keyword FTS5) ---

async function handleKeywordSearch(request: Request, env: Env): Promise<Response> {
  const body = await request.json() as KeywordSearchRequest;

  if (!body.query || !body.project_id) {
    return errorResponse('Missing query or project_id');
  }

  const topK = Math.min(body.top_k ?? DEFAULT_TOP_K, MAX_TOP_K);
  const offset = body.offset ?? 0;

  const { chunks, total } = await tursoKeywordSearch(env, body.query, body.project_id, topK, offset);

  return jsonResponse({
    chunks,
    total_matches: total,
    has_more: total > offset + chunks.length,
  });
}

// --- Relationships ---

async function handleRelationships(request: Request, env: Env): Promise<Response> {
  const body = await request.json() as RelationshipRequest;

  if (!body.symbol || !body.project_id) {
    return errorResponse('Missing symbol or project_id');
  }

  const depth = Math.min(body.depth ?? 2, MAX_RELATIONSHIP_DEPTH);
  const relTypes = body.rel_types ?? ['calls', 'called_by', 'imports', 'exports', 'data_flow'];

  const result = await traverseRelationships(env, body.project_id, body.symbol, depth, relTypes);

  return jsonResponse(result);
}

// --- Projects ---

async function handleListProjects(env: Env): Promise<Response> {
  const projects = await listProjects(env);
  return jsonResponse({ projects });
}

async function handleProjectStatus(projectId: string, env: Env): Promise<Response> {
  const status = await getProjectStatus(env, projectId);
  if (!status) {
    return errorResponse('Project not found', 404);
  }
  return jsonResponse(status);
}

async function handleDeleteProject(projectId: string, env: Env): Promise<Response> {
  const deleted = await deleteProjectChunks(env, projectId);

  return jsonResponse({ deleted_chunks: deleted, project_id: projectId });
}
