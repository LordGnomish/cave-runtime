// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! cave-embed HTTP server — OpenAI-compatible embeddings + rerank.

use clap::Parser;
use std::sync::Arc;

#[derive(Parser, Debug)]
#[command(name = "cave-embed", version, about = "OpenAI-compatible embeddings + rerank server")]
struct Args {
    /// Address to bind, e.g. 127.0.0.1:7997.
    #[arg(long, env = "CAVE_EMBED_ADDR", default_value = "127.0.0.1:7997")]
    addr: String,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = Args::parse();
    let state = Arc::new(cave_embed::EmbedState::default());
    let app = cave_embed::router(state);
    let listener = tokio::net::TcpListener::bind(&args.addr).await?;
    println!(
        "cave-embed {} listening on http://{}",
        cave_embed::UPSTREAM_VERSION,
        args.addr
    );
    axum::serve(listener, app).await?;
    Ok(())
}
