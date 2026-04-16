//! `cave llm` subcommand helpers — catalog browsing, GGUF download,
//! and chat payload construction.

use anyhow::{Context, Result};
use cave_llm_gateway::embedded::{default_model_path, lookup, CATALOG};
use colored::Colorize;
use futures::StreamExt;
use serde_json::{json, Value};
use std::path::PathBuf;
use tokio::io::AsyncWriteExt;

use crate::client::ApiClient;

pub async fn catalog(_c: &ApiClient) -> Result<()> {
    println!("  {}", "AVAILABLE GGUF MODELS".bold());
    println!();
    println!(
        "  {:<24}  {:>8}  {:>8}  {}",
        "ID".bold(),
        "SIZE".bold(),
        "CTX".bold(),
        "DESCRIPTION".bold()
    );
    println!("  {}", "-".repeat(80).dimmed());
    for entry in CATALOG {
        let path = default_model_path(entry.filename);
        let installed = if path.exists() {
            format!(" {}", "[installed]".green())
        } else {
            String::new()
        };
        println!(
            "  {:<24}  {:>6} MB  {:>8}  {}{}",
            entry.id.cyan(),
            entry.approx_size_mb,
            entry.context,
            entry.description,
            installed,
        );
    }
    println!();
    println!("  Run {} to download.", "cave llm pull <id>".bold());
    Ok(())
}

pub async fn pull(id: &str, force: bool) -> Result<()> {
    let entry = lookup(id).with_context(|| {
        format!(
            "Unknown model id `{id}`. Run `cave llm catalog` to see available models."
        )
    })?;

    let dest = default_model_path(entry.filename);
    if dest.exists() && !force {
        println!(
            "  {} {} already exists ({} MB). Use --force to re-download.",
            "✓".green().bold(),
            dest.display(),
            entry.approx_size_mb
        );
        return Ok(());
    }
    ensure_parent(&dest)?;

    println!(
        "  {} {} ({} MB)",
        "↓".cyan().bold(),
        entry.id.cyan(),
        entry.approx_size_mb
    );
    println!("    from {}", entry.url.dimmed());
    println!("    to   {}", dest.display().to_string().dimmed());
    println!();

    let client = reqwest::Client::builder()
        .user_agent("cave-cli/llm-pull")
        .build()?;
    let resp = client
        .get(entry.url)
        .send()
        .await
        .with_context(|| format!("download {} failed", entry.url))?;
    if !resp.status().is_success() {
        anyhow::bail!("download failed: HTTP {}", resp.status());
    }
    let total_bytes = resp.content_length();

    let tmp = dest.with_extension("gguf.partial");
    let mut file = tokio::fs::File::create(&tmp)
        .await
        .with_context(|| format!("create {}", tmp.display()))?;

    let mut stream = resp.bytes_stream();
    let mut downloaded: u64 = 0;
    let mut last_print = std::time::Instant::now();

    while let Some(chunk) = stream.next().await {
        let chunk = chunk.context("stream read")?;
        file.write_all(&chunk).await.context("write chunk")?;
        downloaded += chunk.len() as u64;
        if last_print.elapsed().as_millis() > 500 {
            print_progress(downloaded, total_bytes);
            last_print = std::time::Instant::now();
        }
    }
    file.flush().await?;
    drop(file);
    tokio::fs::rename(&tmp, &dest)
        .await
        .with_context(|| format!("rename {} -> {}", tmp.display(), dest.display()))?;

    print_progress(downloaded, total_bytes);
    println!();
    println!("  {} downloaded to {}", "✓".green().bold(), dest.display());
    println!();
    println!(
        "  Restart the runtime to pick up the new model, or update its config to point at this file."
    );
    Ok(())
}

pub fn build_chat_payload(
    model: &str,
    prompt: &str,
    system: Option<&str>,
    max_tokens: u32,
) -> Value {
    let mut messages: Vec<Value> = Vec::new();
    if let Some(sys) = system {
        messages.push(json!({ "role": "system", "content": sys }));
    }
    messages.push(json!({ "role": "user", "content": prompt }));
    json!({
        "model": model,
        "messages": messages,
        "max_tokens": max_tokens,
        "stream": false,
    })
}

fn ensure_parent(path: &PathBuf) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("create dir {}", parent.display()))?;
    }
    Ok(())
}

fn print_progress(downloaded: u64, total: Option<u64>) {
    let mb = downloaded as f64 / 1_048_576.0;
    match total {
        Some(t) if t > 0 => {
            let pct = (downloaded as f64 / t as f64) * 100.0;
            let total_mb = t as f64 / 1_048_576.0;
            print!("\r    {:>6.1} / {:>6.1} MB  ({:>5.1}%)", mb, total_mb, pct);
        }
        _ => print!("\r    {:>6.1} MB", mb),
    }
    use std::io::Write;
    let _ = std::io::stdout().flush();
}
