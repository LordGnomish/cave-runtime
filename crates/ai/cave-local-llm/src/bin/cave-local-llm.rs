// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! cave-local-llm CLI — offline draft generator for Cave Runtime.
//!
//! Usage:
//!   cave-local-llm run --crate cave-secrets [--dry-run] [--model <model>]

use anyhow::{Context, Result};
use cave_local_llm::{
    draft::{
        Draft, DraftFrontmatter, DraftStatus, build_ollama_prompt, draft_filename,
        parse_ollama_response,
    },
    manifest::{find_missing_functions, read_crate_manifest},
    metrics::DraftMetrics,
    ollama::{GenerateRequest, OllamaClient},
    vllm_paged_attention::{AllocStatus, BlockSpaceManager},
    vllm_quant::{ActivationScheme, Fp8Format, QuantConfig},
    vllm_sampling::{OpenAiSampling, SamplingParams},
};
use clap::{Parser, Subcommand};
use prometheus_client::registry::Registry;
use std::path::PathBuf;
use tracing::{info, warn};

#[derive(Parser)]
#[command(
    name = "cave-local-llm",
    about = "Cave Local LLM — offline draft generation driven by parity manifests",
    version
)]
struct Cli {
    /// Ollama server URL
    #[arg(long, default_value = "http://localhost:11434", env = "OLLAMA_URL")]
    ollama_url: String,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Generate a draft for the first missing function in a crate's parity manifest
    Run {
        /// Target crate (e.g. cave-secrets)
        #[arg(long = "crate")]
        crate_name: String,

        /// Workspace root directory (defaults to current directory)
        #[arg(long, default_value = ".")]
        workspace_root: PathBuf,

        /// Ollama model to use for generation
        #[arg(long, default_value = cave_local_llm::ollama::DEFAULT_MODEL)]
        model: String,

        /// Print the draft to stdout without writing to docs/drafts/
        #[arg(long)]
        dry_run: bool,
    },

    /// Introspect the vLLM-parity inference engine (PagedAttention / quant /
    /// sampling) — pure-Rust ports of vllm-project/vllm (Apache-2.0).
    Vllm {
        #[command(subcommand)]
        cmd: VllmCmd,
    },
}

#[derive(Subcommand)]
enum VllmCmd {
    /// PagedAttention block-pool admission for a prompt of N tokens
    Paged {
        /// Total GPU KV blocks
        #[arg(long, default_value_t = 256)]
        gpu_blocks: usize,
        /// Tokens per block
        #[arg(long, default_value_t = 16)]
        block_size: usize,
        /// Prompt length in tokens
        #[arg(long)]
        prompt_tokens: usize,
    },
    /// Weight-quantization footprint vs fp16 for a linear layer
    Quant {
        /// Scheme: awq | gptq | fp8
        #[arg(long, default_value = "awq")]
        method: String,
        /// Weight bit width (awq=4; gptq=2/3/4/8; fp8=8)
        #[arg(long, default_value_t = 4)]
        bits: u32,
        /// Group size (-1 = per-channel)
        #[arg(long, default_value_t = 128)]
        group_size: i32,
        /// Input features
        #[arg(long, default_value_t = 4096)]
        in_features: usize,
        /// Output features
        #[arg(long, default_value_t = 4096)]
        out_features: usize,
    },
    /// Validate sampling parameters and report the sampling type
    Sample {
        /// Softmax temperature (0 = greedy)
        #[arg(long, default_value_t = 1.0)]
        temperature: f32,
        /// Nucleus top_p
        #[arg(long, default_value_t = 1.0)]
        top_p: f32,
        /// Top-k (-1 disables)
        #[arg(long, default_value_t = -1)]
        top_k: i32,
        /// Number of output sequences
        #[arg(long, default_value_t = 1)]
        n: usize,
        /// Max generated tokens
        #[arg(long, default_value_t = 16)]
        max_tokens: usize,
    },
}

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env()
                .add_directive("cave_local_llm=info".parse().unwrap()),
        )
        .init();

    let cli = Cli::parse();
    if let Err(e) = run(cli).await {
        eprintln!("error: {:#}", e);
        std::process::exit(1);
    }
}

async fn run(cli: Cli) -> Result<()> {
    let mut registry = Registry::default();
    let metrics = DraftMetrics::new(&mut registry);

    match cli.command {
        Commands::Run {
            crate_name,
            workspace_root,
            model,
            dry_run,
        } => {
            let workspace_root = workspace_root.canonicalize().with_context(|| {
                format!("workspace root '{}' not found", workspace_root.display())
            })?;

            let start = std::time::Instant::now();

            let (manifest, crate_root) = read_crate_manifest(&crate_name, &workspace_root)
                .with_context(|| format!("reading parity manifest for {crate_name}"))?;

            let missing = find_missing_functions(&manifest, &crate_root);
            if missing.is_empty() {
                println!("No missing functions found for {crate_name} — parity complete!");
                return Ok(());
            }

            let target = &missing[0];
            info!(
                crate = %crate_name,
                local_fn = %target.local_name,
                upstream_fn = %target.upstream_name,
                total_missing = missing.len(),
                "generating draft",
            );

            if missing.len() > 1 {
                warn!(
                    remaining = missing.len() - 1,
                    "additional missing functions will be addressed in subsequent runs",
                );
            }

            let upstream_repo = match manifest.primary_upstream() {
                Some(u) => format!("{}/{}", u.org, u.repo),
                None => "(no-upstream-declared)".to_string(),
            };

            let prompt = build_ollama_prompt(
                &crate_name,
                &target.local_name,
                &upstream_repo,
                &target.upstream_name,
                &target.file,
            );

            let ollama = OllamaClient::new(&cli.ollama_url);
            let req = GenerateRequest {
                model: model.clone(),
                prompt,
                stream: Some(false),
                options: None,
                keep_alive: None,
                images: None,
            };

            let response = ollama.generate(req).await.map_err(|e| {
                metrics.drafts_failed_total.inc();
                anyhow::anyhow!("Ollama generate failed: {e}")
            })?;

            let elapsed = start.elapsed().as_secs_f64();
            metrics.draft_duration_seconds.observe(elapsed);

            let (test_code, skel_code) = parse_ollama_response(&response.response);

            let fm = DraftFrontmatter {
                crate_name: crate_name.clone(),
                upstream_repo: upstream_repo.clone(),
                upstream_file: target.file.clone(),
                upstream_fn: target.upstream_name.clone(),
                status: DraftStatus::Draft,
                tier: 1,
                created_at: chrono::Utc::now(),
            };
            let draft = Draft {
                frontmatter: fm,
                failing_test: test_code,
                skeleton: skel_code,
            };
            let rendered = draft.render();

            if dry_run {
                println!("{rendered}");
                return Ok(());
            }

            let drafts_dir = workspace_root.join("docs").join("drafts");
            std::fs::create_dir_all(&drafts_dir)
                .with_context(|| format!("creating {}", drafts_dir.display()))?;

            let filename = draft_filename(&crate_name, &target.local_name, chrono::Utc::now());
            let out_path = drafts_dir.join(&filename);
            std::fs::write(&out_path, &rendered)
                .with_context(|| format!("writing draft to {}", out_path.display()))?;

            metrics.drafts_generated_total.inc();

            println!("Draft written → {}", out_path.display());
            println!("  crate:    {crate_name}");
            println!(
                "  function: {} → {}",
                target.upstream_name, target.local_name
            );
            println!("  model:    {model}");
            println!("  elapsed:  {elapsed:.2}s");

            Ok(())
        }

        Commands::Vllm { cmd } => run_vllm(cmd),
    }
}

/// Handle `cave-local-llm vllm …` engine-introspection subcommands.
fn run_vllm(cmd: VllmCmd) -> Result<()> {
    match cmd {
        VllmCmd::Paged {
            gpu_blocks,
            block_size,
            prompt_tokens,
        } => {
            let m = BlockSpaceManager::new(block_size, gpu_blocks, 0, 0.01);
            let need = BlockSpaceManager::blocks_for_tokens(prompt_tokens, block_size);
            let status = match m.can_allocate(prompt_tokens) {
                AllocStatus::Ok => "OK (admit now)",
                AllocStatus::Later => "LATER (retry after eviction)",
                AllocStatus::Never => "NEVER (exceeds pool)",
            };
            println!("PagedAttention admission");
            println!("  pool:          {gpu_blocks} blocks × {block_size} tokens");
            println!("  prompt:        {prompt_tokens} tokens → {need} blocks");
            println!("  free blocks:   {}", m.num_free_gpu_blocks());
            println!("  decision:      {status}");
            Ok(())
        }
        VllmCmd::Quant {
            method,
            bits,
            group_size,
            in_features,
            out_features,
        } => {
            let cfg = match method.as_str() {
                "awq" => QuantConfig::awq(bits, group_size),
                "gptq" => QuantConfig::gptq(bits, group_size, false, true),
                "fp8" => Ok(QuantConfig::fp8(Fp8Format::E4M3, ActivationScheme::Dynamic)),
                other => return Err(anyhow::anyhow!("unknown quant method '{other}'")),
            }
            .map_err(|e| anyhow::anyhow!("{e}"))?;
            let ratio = cfg.compression_ratio(in_features, out_features);
            let fp16 = QuantConfig::fp16_bytes(in_features, out_features);
            let q = cfg.quantized_bytes(in_features, out_features);
            println!("{:?} quantization ({in_features}×{out_features})", cfg.method);
            println!("  weight bits:   {}", cfg.weight_bits);
            println!("  pack factor:   {} weights / int32", cfg.pack_factor());
            println!("  fp16 bytes:    {fp16}");
            println!("  quant bytes:   {q}");
            println!("  compression:   {:.1}% of fp16", ratio * 100.0);
            Ok(())
        }
        VllmCmd::Sample {
            temperature,
            top_p,
            top_k,
            n,
            max_tokens,
        } => {
            let req = OpenAiSampling {
                temperature: Some(temperature),
                top_p: Some(top_p),
                top_k: Some(top_k),
                n: Some(n),
                max_tokens: Some(max_tokens),
                ..Default::default()
            };
            match SamplingParams::from_openai(req) {
                Ok(p) => {
                    println!("sampling params valid");
                    println!("  type:        {:?}", p.sampling_type());
                    println!("  temperature: {}", p.temperature);
                    println!("  top_p:       {}", p.top_p);
                    println!("  top_k:       {}", p.top_k);
                    println!("  n:           {}", p.n);
                    println!("  max_tokens:  {:?}", p.max_tokens);
                    Ok(())
                }
                Err(e) => Err(anyhow::anyhow!("invalid sampling params: {e}")),
            }
        }
    }
}
