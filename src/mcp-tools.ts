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
          description: 'Advanced relationship traversal configuration',
          properties: {
            enabled: { type: 'boolean', default: true },
            max_hops: { type: 'number', default: 2, minimum: 1, maximum: 5 },
            relationship_types: {
              type: 'array',
              items: { 
                type: 'string', 
                enum: ['calls', 'imports', 'exports', 'data_flow', 'co_change', 'extends', 'implements', 'throws', 'catches', 'depends_on'] 
              },
              default: ['calls', 'imports']
            },
            hop_decay: { type: 'number', default: 0.8, minimum: 0.1, maximum: 1.0 },
            focus_symbols: {
              type: 'array',
              description: 'Specific symbols to focus traversal on',
              items: { type: 'string' }
            },
            traversal_direction: {
              type: 'string',
              enum: ['forward', 'backward', 'both'],
              default: 'both',
              description: 'Direction of relationship traversal'
            },
            min_strength: {
              type: 'number',
              default: 0.3,
              minimum: 0.1,
              maximum: 1.0,
              description: 'Minimum relationship strength threshold'
            },
            include_paths: {
              type: 'boolean',
              default: true,
              description: 'Include relationship paths in response'
            }
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
  },

  relationship_analysis: {
    name: 'relationship_analysis',
    description: 'Advanced code relationship analysis and traversal for understanding complex code interactions',
    inputSchema: {
      type: 'object',
      properties: {
        analysis_type: {
          type: 'string',
          enum: ['call_graph', 'dependency_chain', 'data_flow', 'error_propagation', 'impact_analysis'],
          description: 'Type of relationship analysis to perform'
        },
        starting_symbols: {
          type: 'array',
          description: 'Starting points for analysis (function names, class names, file paths)',
          items: { type: 'string' },
          minItems: 1
        },
        target_symbols: {
          type: 'array',
          description: 'Target symbols to find paths to (optional)',
          items: { type: 'string' }
        },
        max_depth: {
          type: 'number',
          default: 3,
          minimum: 1,
          maximum: 10,
          description: 'Maximum traversal depth'
        },
        relationship_filters: {
          type: 'array',
          description: 'Types of relationships to include in analysis',
          items: { 
            type: 'string', 
            enum: ['calls', 'imports', 'exports', 'data_flow', 'extends', 'implements', 'throws', 'catches', 'depends_on'] 
          },
          default: ['calls', 'imports', 'data_flow']
        },
        include_strength_scores: {
          type: 'boolean',
          default: true,
          description: 'Include relationship strength and confidence scores'
        },
        visualization_format: {
          type: 'string',
          enum: ['text', 'mermaid', 'json'],
          default: 'text',
          description: 'Format for relationship visualization'
        },
        context_radius: {
          type: 'number',
          default: 3,
          minimum: 0,
          maximum: 10,
          description: 'Lines of code context around each discovered symbol'
        }
      },
      required: ['analysis_type', 'starting_symbols']
    }
  },

  trace_execution_path: {
    name: 'trace_execution_path',
    description: 'Trace execution paths through code to understand flow and dependencies',
    inputSchema: {
      type: 'object',
      properties: {
        entry_point: {
          type: 'string',
          description: 'Starting function or method for tracing'
        },
        trace_type: {
          type: 'string',
          enum: ['forward_trace', 'backward_trace', 'bidirectional'],
          default: 'forward_trace',
          description: 'Direction of execution tracing'
        },
        include_data_flow: {
          type: 'boolean',
          default: true,
          description: 'Include data flow analysis in trace'
        },
        include_error_paths: {
          type: 'boolean',
          default: true,
          description: 'Include error handling and exception paths'
        },
        max_execution_depth: {
          type: 'number',
          default: 5,
          minimum: 1,
          maximum: 15,
          description: 'Maximum depth of execution trace'
        },
        filter_by_files: {
          type: 'array',
          description: 'Limit trace to specific files or patterns',
          items: { type: 'string' }
        },
        output_format: {
          type: 'string',
          enum: ['detailed', 'summary', 'flowchart'],
          default: 'detailed',
          description: 'Level of detail in trace output'
        }
      },
      required: ['entry_point']
    }
  },

  find_code_patterns: {
    name: 'find_code_patterns',
    description: 'Find complex code patterns and architectural relationships',
    inputSchema: {
      type: 'object',
      properties: {
        pattern_type: {
          type: 'string',
          enum: ['design_pattern', 'anti_pattern', 'architectural_pattern', 'dependency_cycle', 'code_smell'],
          description: 'Type of pattern to search for'
        },
        pattern_description: {
          type: 'string',
          description: 'Natural language description of the pattern to find'
        },
        scope: {
          type: 'string',
          enum: ['file', 'module', 'package', 'entire_codebase'],
          default: 'entire_codebase',
          description: 'Scope of pattern search'
        },
        confidence_threshold: {
          type: 'number',
          default: 0.7,
          minimum: 0.1,
          maximum: 1.0,
          description: 'Minimum confidence for pattern matches'
        },
        include_examples: {
          type: 'boolean',
          default: true,
          description: 'Include code examples of found patterns'
        },
        max_results: {
          type: 'number',
          default: 10,
          minimum: 1,
          maximum: 50,
          description: 'Maximum number of pattern instances to return'
        }
      },
      required: ['pattern_type']
    }
  },

  real_time_status: {
    name: 'real_time_status',
    description: 'Get real-time file watching status and context freshness for the codebase',
    inputSchema: {
      type: 'object',
      properties: {},
      required: []
    }
  }
} as const;

export type CortexToolName = keyof typeof CORTEX_TOOLS;