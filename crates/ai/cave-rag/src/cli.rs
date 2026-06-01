// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! `cave-rag` command-line surface.
//!
//! A thin, offline CLI over the library: extract a knowledge graph, run a
//! local graph search, or split text into chunks. Every command emits a JSON
//! document on stdout so the output composes with `jq` and the portal.
//!
//! The bin entry point ([`main`](../bin/cave-rag)) is a one-liner over
//! [`run_args`]; all logic lives here so it is unit-testable without spawning
//! a process.

use std::ffi::OsString;

use clap::{Parser, Subcommand};
use serde_json::json;

use crate::error::{RagError, Result};
use crate::graphrag::extract_graph;
use crate::splitter::RecursiveCharacterTextSplitter;

/// `cave-rag` — offline retrieval-augmented-generation toolbox.
#[derive(Debug, Parser)]
#[command(name = "cave-rag", version, about)]
pub struct Cli {
    /// The subcommand to run.
    #[command(subcommand)]
    pub command: Command,
}

/// Top-level commands.
#[derive(Debug, Subcommand)]
pub enum Command {
    /// Knowledge-graph operations (GraphRAG).
    Graph {
        /// The graph action.
        #[command(subcommand)]
        action: GraphAction,
    },
    /// Split text into chunks (recursive character splitter).
    Split {
        /// The text to split.
        #[arg(long)]
        text: String,
        /// Target chunk size in characters.
        #[arg(long, default_value_t = 400)]
        size: usize,
        /// Overlap between adjacent chunks.
        #[arg(long, default_value_t = 0)]
        overlap: usize,
    },
}

/// `graph` subcommands.
#[derive(Debug, Subcommand)]
pub enum GraphAction {
    /// Extract entities, relationships and communities from text.
    Extract {
        /// The source text.
        #[arg(long)]
        text: String,
    },
    /// Local search: gather the neighborhood of entities named in the query.
    Search {
        /// The source text to build the graph from.
        #[arg(long)]
        text: String,
        /// The natural-language query naming seed entities.
        #[arg(long)]
        query: String,
        /// How many hops to expand from each seed.
        #[arg(long, default_value_t = 1)]
        hops: usize,
    },
}

/// Parse `args` and run the resulting command, returning its JSON output.
///
/// Parse failures (unknown command, bad flags) surface as
/// [`RagError`](crate::RagError) rather than exiting the process, so callers —
/// and tests — stay in control.
pub fn run_args<I, T>(args: I) -> Result<String>
where
    I: IntoIterator<Item = T>,
    T: Into<OsString> + Clone,
{
    let cli = Cli::try_parse_from(args).map_err(|e| RagError::Loader(e.to_string()))?;
    run(cli)
}

/// Execute a parsed [`Cli`] and return its JSON output.
pub fn run(cli: Cli) -> Result<String> {
    match cli.command {
        Command::Graph { action } => run_graph(action),
        Command::Split {
            text,
            size,
            overlap,
        } => {
            let splitter = RecursiveCharacterTextSplitter::default()
                .with_chunk_size(size)
                .with_chunk_overlap(overlap);
            let chunks = splitter.split_text(&text);
            Ok(json!({
                "chunk_count": chunks.len(),
                "chunks": chunks,
            })
            .to_string())
        }
    }
}

fn run_graph(action: GraphAction) -> Result<String> {
    match action {
        GraphAction::Extract { text } => {
            let g = extract_graph(&text);
            let communities = g.communities();
            // A stable, de-duplicated entity list (communities partition all
            // entities, so flattening them covers the graph exactly once).
            let mut all_entities: Vec<String> =
                communities.iter().flatten().cloned().collect();
            all_entities.sort();
            all_entities.dedup();
            let relationships: Vec<serde_json::Value> = g
                .relationships()
                .iter()
                .map(|r| json!({"source": r.source, "relation": r.relation, "target": r.target}))
                .collect();
            Ok(json!({
                "entities": all_entities,
                "entity_count": g.entity_count(),
                "relationships": relationships,
                "relationship_count": g.relationship_count(),
                "communities": communities,
            })
            .to_string())
        }
        GraphAction::Search { text, query, hops } => {
            let g = extract_graph(&text);
            let context = g.local_search(&query, hops);
            Ok(json!({
                "query": query,
                "hops": hops,
                "context": context,
            })
            .to_string())
        }
    }
}
