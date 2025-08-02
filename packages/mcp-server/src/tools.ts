// MCP tool definitions and schemas

export const CORTEX_TOOLS = {
  semantic_search: {
    name: 'semantic_search',
    description: 'Semantic code search using vector embeddings to find relevant code chunks',
    inputSchema: {
      type: 'object',
      properties: {
        query: {
          type: 'string',
          description: 'Natural language description of what you\'re looking for'
        },
        max_chunks: {
          type: 'number',
          description: 'Maximum number of code chunks to return',
          default: 20,
          minimum: 1,
          maximum: 100
        },
        file_filters: {
          type: 'array',
          description: 'Glob patterns to filter files (e.g., ["*.js", "src/**"])',
          items: { type: 'string' }
        },
        include_related: {
          type: 'boolean',
          description: 'Include semantically related code chunks',
          default: true
        },
        multi_hop: {
          type: 'object',
          description: 'Multi-hop relationship traversal configuration',
          properties: {
            enabled: { type: 'boolean', default: true },
            max_hops: { type: 'number', default: 2, minimum: 1, maximum: 5 },
            relationship_types: {
              type: 'array',
              items: { type: 'string', enum: ['calls', 'imports', 'data_flow', 'co_change'] },
              default: ['calls']
            },
            hop_decay: { type: 'number', default: 0.8, minimum: 0.1, maximum: 1.0 }
          }
        }
      },
      required: ['query']
    }
  },

  contextual_read: {
    name: 'contextual_read',
    description: 'Read a specific file with additional semantic context',
    inputSchema: {
      type: 'object',
      properties: {
        file_path: {
          type: 'string',
          description: 'Path to the file to read'
        },
        semantic_context: {
          type: 'string',
          description: 'What aspect of the file you\'re interested in'
        },
        max_context_tokens: {
          type: 'number',
          description: 'Maximum tokens to include in context',
          default: 2000,
          minimum: 500,
          maximum: 8000
        }
      },
      required: ['file_path']
    }
  },

  code_intelligence: {
    name: 'code_intelligence',
    description: 'High-level semantic analysis for complex development tasks',
    inputSchema: {
      type: 'object',
      properties: {
        task: {
          type: 'string',
          description: 'High-level description of the development task'
        },
        focus_areas: {
          type: 'array',
          description: 'Specific subsystems or modules to focus on',
          items: { type: 'string' }
        },
        recency_weight: {
          type: 'number',
          description: 'Weight to give to recently modified code (0-1)',
          default: 0.3,
          minimum: 0,
          maximum: 1
        },
        max_context_tokens: {
          type: 'number',
          description: 'Maximum tokens for the complete context package',
          default: 4000,
          minimum: 1000,
          maximum: 16000
        }
      },
      required: ['task']
    }
  }
} as const;

export type CortexToolName = keyof typeof CORTEX_TOOLS;