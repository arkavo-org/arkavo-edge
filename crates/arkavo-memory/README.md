# Arkavo Memory

A local-first, privacy-focused memory service for AI agents with fast vector similarity search and native Rust embeddings. This service is integrated into the main `arkavo` binary and exposed through MCP (Model Context Protocol) tools.

## Features

- 100% local storage with SQLite persistence
- Fast semantic search using HNSW (Hierarchical Navigable Small World) algorithm
- Text embeddings via fastembed (Rust-native, no external dependencies)
- MCP tool integration - no HTTP server needed
- Automatic memory categorization
- Flexible metadata support
- **Zero configuration required** - all settings have sensible defaults
- **Integrated into arkavo** - no separate server needed
- **Self-contained** - embedding models are automatically downloaded on first use

## Architecture

The memory service uses a hybrid approach:
- **SQLite**: Persistent storage for memory content, metadata, and serialized embeddings
- **HNSW Index**: In-memory vector index for ultra-fast similarity search
- **HashMap**: O(1) lookup by memory ID
- **fastembed**: Rust-native library for generating text embeddings
- **MCP Tools**: Exposed as MCP tools instead of HTTP endpoints

## Zero Configuration

The service works out of the box with no configuration required:
- Database is automatically created in `.arkavo/memory_server/` relative to where you run arkavo
- Uses `AllMiniLML6V2` embedding model by default
- Embedding model is automatically downloaded on first use (~30MB) to `.arkavo/fastembed_cache/`
- No external services required
- All data stays in the `.arkavo/` directory

## Integration with Arkavo

The memory service is automatically available when you use:

```bash
# Chat command - memory tools available via MCP
arkavo chat

# Serve command (MCP mode) - memory tools exposed to external clients
arkavo serve
```

## MCP Tools

The following MCP tools are available:

### store_memory
Store a memory with automatic embedding generation
```json
{
  "tool": "store_memory",
  "arguments": {
    "content": "Memory content to store",
    "metadata": { "optional": "json metadata" },
    "category": "optional category"
  }
}
```

### search_memory
Search memories using semantic similarity
```json
{
  "tool": "search_memory",
  "arguments": {
    "query": "Search query",
    "limit": 10,
    "category": "optional category filter"
  }
}
```

### get_memory
Retrieve a specific memory by ID
```json
{
  "tool": "get_memory",
  "arguments": {
    "id": "memory-uuid-here"
  }
}
```

### categorize_memory
Categorize content based on existing memories
```json
{
  "tool": "categorize_memory",
  "arguments": {
    "content": "Text to categorize"
  }
}
```

## Usage Examples

In an arkavo chat session:

```
> @store_memory "The capital of France is Paris" {"source": "geography"} "facts"
Memory stored successfully with ID: 123e4567-e89b-12d3-a456-426614174000

> @search_memory "What is the capital of France?" 5
Found 1 result:
- "The capital of France is Paris" (score: 0.95)

> @categorize_memory "The Eiffel Tower is 330 meters tall"
Category: facts (confidence: 0.87)
```

## Performance

- HNSW provides sub-linear search complexity (~O(log n))
- Configurable parameters for speed/recall tradeoff
- Parallel search support for batch queries
- In-memory index for fast access

## Prerequisites

None! The embedding model will be automatically downloaded on first use.

## Testing

```bash
cargo test --package arkavo-memory
```

Note: Tests are marked as ignored by default. The embedding model will be downloaded during test execution if not already cached.