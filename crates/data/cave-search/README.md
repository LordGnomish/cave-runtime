# cave-search

High-performance, sovereign search engine for the Cloud OS.

## Status

This crate is currently in the pre-open-source-launch phase. Feature parity with the upstream reference implementation is actively tracked and being implemented incrementally.

## Upstream

- [cave-search upstream](https://github.com/cave-os/cave-search)

## Surface ported

- Full-text indexing with support for complex document structures.
- Inverted index construction optimized for Rust memory safety.
- Real-time search queries with low-latency response times.
- Support for boolean operators (AND, OR, NOT) in query parsing.
- Phrase matching and proximity searches for precise results.
- Configurable ranking algorithms based on term frequency.
- Automatic index optimization and merging for performance.
- Thread-safe concurrent access to the search index.
- Serialization support for persistent index storage.
- Integration with the cave-runtime logging subsystem.

## Public API

- `pub struct Index`: The main entry point for managing search indices.
- `pub fn add_document`: Adds a new document to the index for retrieval.
- `pub fn search`: Executes a query against the current index state.
- `pub struct Query`: Represents a parsed search query structure.
- `pub struct SearchResult`: Contains ranked results and metadata.
- `pub fn optimize`: Triggers index optimization to reclaim space.

## Tests

Comprehensive unit tests cover index construction, query parsing, and ranking logic. Integration tests verify end-to-end search performance and consistency under concurrent load.

## License

Apache-2.0

## See also

- [../cave-index](../cave-index)
- [../cave-query](../cave-query)
- [../cave-storage](../cave-storage)
