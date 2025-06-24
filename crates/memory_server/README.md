# Memory Server

A local-first, privacy-focused memory server for AI agents with fast vector similarity search and Ollama integration.

## Features

- 100% local storage with SQLite persistence
- Fast semantic search using HNSW (Hierarchical Navigable Small World) algorithm
- Text embeddings via local Ollama instance
- RESTful API built with Actix-Web
- Automatic memory categorization
- Flexible metadata support
- **Zero configuration required** - all settings have sensible defaults

## Architecture

The memory server uses a hybrid approach:
- **SQLite**: Persistent storage for memory content, metadata, and serialized embeddings
- **HNSW Index**: In-memory vector index for ultra-fast similarity search
- **HashMap**: O(1) lookup by memory ID
- **Ollama**: Local LLM for generating text embeddings

## Zero Configuration

The server works out of the box with no configuration required:
- Database is automatically created in the user's data directory (`~/Library/Application Support/arkavo/memory_server/` on macOS)
- Ollama is expected to run on `localhost:11434` (standard installation)
- Uses `nomic-embed-text` model by default
- Automatically checks for Ollama availability on startup

## API Endpoints

### Store Memory
```
POST /memory
Content-Type: application/json

{
  "content": "Memory content to store",
  "metadata": { "optional": "json metadata" },
  "category": "optional category"
}
```

### Search Memories
```
POST /memory/search
Content-Type: application/json

{
  "query": "Search query",
  "limit": 10,
  "category": "optional category filter"
}
```

### Get Memory by ID
```
GET /memory/{id}
```

### Categorize Memory
```
POST /memory/categorize
Content-Type: application/json

{
  "content": "Text to categorize"
}
```

## Performance

- HNSW provides sub-linear search complexity (~O(log n))
- Configurable parameters for speed/recall tradeoff
- Parallel search support for batch queries
- Index persistence to avoid rebuild on restart

## Running the Server

```bash
# First time setup - pull the embedding model
ollama pull nomic-embed-text

# Run the memory server (no configuration needed)
cargo run --bin memory_server
```

The server will:
1. Check if Ollama is running
2. Verify the embedding model is available
3. Create data directory automatically
4. Start on `http://localhost:8080`

## Testing

```bash
cargo test --package memory_server
```

Note: Tests require a running Ollama instance with the embedding model installed.