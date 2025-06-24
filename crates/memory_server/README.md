# Memory Server

A local-first, privacy-focused memory server for AI agents with fast vector similarity search and Ollama integration.

## Features

- 100% local storage with SQLite persistence
- Fast semantic search using HNSW (Hierarchical Navigable Small World) algorithm
- Text embeddings via local Ollama instance
- RESTful API built with Actix-Web
- Automatic memory categorization
- Flexible metadata support

## Architecture

The memory server uses a hybrid approach:
- **SQLite**: Persistent storage for memory content, metadata, and serialized embeddings
- **HNSW Index**: In-memory vector index for ultra-fast similarity search
- **HashMap**: O(1) lookup by memory ID
- **Ollama**: Local LLM for generating text embeddings

## Configuration

Environment variables:
- `DATABASE_URL`: SQLite database path (default: `sqlite:memories.db`)
- `OLLAMA_BASE_URL`: Ollama API endpoint (default: `http://localhost:11434`)
- `OLLAMA_EMBEDDING_MODEL`: Model for embeddings (default: `nomic-embed-text`)

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
# Ensure Ollama is running with an embedding model
ollama pull nomic-embed-text

# Run the memory server
cargo run --bin memory_server
```

The server will start on `http://localhost:8080`.

## Testing

```bash
cargo test --package memory_server
```

Note: Tests require a running Ollama instance with the embedding model installed.