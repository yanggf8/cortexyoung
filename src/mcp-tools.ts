// MCP tool definitions and schemas

export const CORTEX_TOOLS = {
  semantic_search: {
    name: 'semantic_search',
    description: 'BEST FOR: Quick code discovery, finding specific functions/patterns, debugging. WHEN TO USE: First choice for most code search needs - fast, efficient, and comprehensive. Uses advanced semantic search with MMR optimization to find the most relevant code chunks while ensuring diversity. Automatically includes related dependencies for complete context. RESPONSE: Optimized for Claude Code with ~80-90% token reduction. Large responses automatically chunked.',
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
        },
        chunk_size: {
          type: 'number',
          description: 'Size of chunks for large responses (default: 20000 characters)',
          default: 20000,
          minimum: 5000,
          maximum: 100000
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
    description: 'BEST FOR: Complex development tasks, architecture understanding, feature implementation. WHEN TO USE: When you need comprehensive analysis of large codebases or complex architectural patterns. Provides advanced semantic analysis with smart dependency chain traversal. CONTEXT OPTIMIZATION: Delivers maximum code understanding with critical set protection and intelligent token budgeting. RESPONSE: MCP-optimized with automatic chunking for large analyses.',
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
    description: 'BEST FOR: Understanding code dependencies, impact analysis, refactoring planning. WHEN TO USE: Before making changes, for impact analysis, or understanding how code components connect. Traces complex code relationships (calls, imports, data flow) with strength scoring and confidence metrics. Essential for safe refactoring and architectural decisions. RESPONSE: MCP-optimized with visualization support and automatic chunking.',
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
    description: 'BEST FOR: Understanding execution flow, debugging error paths, tracing function calls. WHEN TO USE: When debugging issues, understanding program flow, or analyzing error propagation. Traces execution paths through code with data flow analysis and error path detection. RESPONSE: MCP-optimized with detailed execution summaries and automatic chunking.',
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
    description: 'BEST FOR: Identifying design patterns, code smells, architectural patterns, anti-patterns. WHEN TO USE: During code reviews, refactoring planning, or architectural analysis. Finds complex code patterns and architectural relationships with confidence scoring. RESPONSE: MCP-optimized with pattern examples and automatic chunking.',
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
    description: 'BEST FOR: Checking if context is up-to-date, verifying file watching status. WHEN TO USE: Before important operations, to check context freshness, or troubleshoot real-time updates. Shows real-time file watching status and context freshness for the codebase. RESPONSE: Lightweight status report, always fast.',
    inputSchema: {
      type: 'object',
      properties: {},
      required: []
    }
  },

  fetch_chunk: {
    name: 'fetch_chunk',
    description: 'CHUNKING TOOL: Retrieves a specific chunk from large responses by index (random access). WHEN TO USE: When a Cortex tool returns "Response too large" with a cacheKey and you need to access a specific chunk number or re-read a previous chunk. Essential for handling large analysis results from code_intelligence and relationship_analysis tools.',
    inputSchema: {
      type: 'object',
      properties: {
        cacheKey: {
          type: 'string',
          description: 'The cache key provided in the "Response too large" message from any Cortex tool (e.g., "4bb2c31e-5259-4c0e-afc2-37a5498260aa")'
        },
        chunkIndex: {
          type: 'number',
          description: 'Which chunk to retrieve (1-based index). For example, if response says "chunk: 1/5", you can request chunks 1, 2, 3, 4, or 5',
          minimum: 1
        }
      },
      required: ['cacheKey', 'chunkIndex']
    }
  },

  next_chunk: {
    name: 'next_chunk',
    description: 'CHUNKING TOOL: Fetches the next chunk in sequence from large responses (sequential access). WHEN TO USE: When a Cortex tool returns "Response too large" with a cacheKey and you want to read through all chunks sequentially. More convenient than fetch_chunk when reading everything in order. Essential for comprehensive analysis of large code intelligence results.',
    inputSchema: {
      type: 'object',
      properties: {
        cacheKey: {
          type: 'string',
          description: 'The cache key from the "Response too large" message from any Cortex tool. This tool automatically tracks which chunk to return next.'
        }
      },
      required: ['cacheKey']
    }
  },

  get_current_project: {
    name: 'get_current_project',
    description: 'PROJECT MANAGEMENT: Shows the current active project that Cortex is analyzing. WHEN TO USE: To verify which project Claude Code is currently working on, or to understand the context before running other tools. Essential for multi-project workflows to ensure you are analyzing the correct codebase.',
    inputSchema: {
      type: 'object',
      properties: {},
      required: []
    }
  },

  list_available_projects: {
    name: 'list_available_projects',
    description: 'PROJECT MANAGEMENT: Lists all discovered projects and their status. WHEN TO USE: To see all available projects that Cortex can analyze, including their indexing status and basic information. Helps with project discovery and management.',
    inputSchema: {
      type: 'object',
      properties: {
        include_stats: {
          type: 'boolean',
          description: 'Include basic statistics for each project (file count, index status)',
          default: true
        }
      },
      required: []
    }
  },

  switch_project: {
    name: 'switch_project',
    description: 'PROJECT MANAGEMENT: Switch Cortex to analyze a different project/repository. WHEN TO USE: When Claude Code needs to work on a different codebase. Automatically handles indexing the new project and updating context. Essential for multi-project development workflows.',
    inputSchema: {
      type: 'object',
      properties: {
        project_path: {
          type: 'string',
          description: 'Absolute path to the project directory to switch to (e.g., "/home/user/my-project")'
        },
        project_name: {
          type: 'string',
          description: 'Optional friendly name for the project. If not provided, will use directory name.'
        },
        auto_index: {
          type: 'boolean',
          description: 'Automatically start indexing if the project is not already indexed',
          default: true
        }
      },
      required: ['project_path']
    }
  },

  add_project: {
    name: 'add_project',
    description: 'PROJECT MANAGEMENT: Add a new project to Cortex for future analysis. WHEN TO USE: To register a new project/repository that Claude Code will work with. Pre-indexes the project for faster future access. Useful for setting up multi-project environments.',
    inputSchema: {
      type: 'object',
      properties: {
        project_path: {
          type: 'string',
          description: 'Absolute path to the project directory to add (e.g., "/home/user/new-project")'
        },
        project_name: {
          type: 'string',
          description: 'Optional friendly name for the project. If not provided, will use directory name.'
        },
        start_indexing: {
          type: 'boolean',
          description: 'Start indexing immediately after adding the project',
          default: true
        }
      },
      required: ['project_path']
    }
  },

  multi_instance_health: {
    name: 'multi_instance_health',
    description: 'HEALTH CHECK: Multi-instance health monitoring and diagnostics. WHEN TO USE: When experiencing MCP connection issues, multiple Claude Code instances, or need to diagnose startup problems. Provides comprehensive health status of all active Cortex MCP instances, session conflicts, resource usage, and troubleshooting recommendations. Essential for debugging multi-instance connection failures.',
    inputSchema: {
      type: 'object',
      properties: {},
      required: []
    }
  }
} as const;

export type CortexToolName = keyof typeof CORTEX_TOOLS;