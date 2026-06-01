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
    vllm_engine::{EngineConfig, FinishReason, LLMEngine, SeqView, StepModel},
    vllm_paged_attention::{AllocStatus, BlockSpaceManager},
    vllm_parallel::{
        attn_heads_per_rank, get_pp_indices, kv_heads_per_rank, vocab_partition, ParallelConfig,
    },
    vllm_prefix_cache::PrefixCachingAllocator,
    vllm_quant::{ActivationScheme, Fp8Format, QuantConfig},
    vllm_sampler,
    vllm_sampling::{OpenAiSampling, SamplingParams},
    vllm_scheduler::ChunkedPrefillPlanner,
    vllm_spec_decode::TypicalAcceptanceSampler,
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
    /// Run the continuous-batching LLMEngine over a constant-token stub model
    Engine {
        /// Prompt length in tokens
        #[arg(long, default_value_t = 8)]
        prompt_tokens: usize,
        /// Max generated tokens
        #[arg(long, default_value_t = 16)]
        max_tokens: usize,
        /// Total GPU KV blocks
        #[arg(long, default_value_t = 256)]
        gpu_blocks: usize,
        /// Tokens per block
        #[arg(long, default_value_t = 16)]
        block_size: usize,
    },
    /// Report the automatic-prefix-cache hit rate for a repeated shared prefix
    Prefix {
        /// Total physical blocks
        #[arg(long, default_value_t = 16)]
        blocks: usize,
        /// Tokens per block
        #[arg(long, default_value_t = 2)]
        block_size: usize,
        /// Number of identical sequences sharing the prefix
        #[arg(long, default_value_t = 4)]
        sequences: usize,
        /// Full blocks per sequence prefix
        #[arg(long, default_value_t = 3)]
        prefix_blocks: usize,
    },
    /// Print the tensor/pipeline-parallel sharding plan for one global rank
    Parallel {
        /// Tensor-parallel size
        #[arg(long, default_value_t = 2)]
        tp: usize,
        /// Pipeline-parallel size
        #[arg(long, default_value_t = 2)]
        pp: usize,
        /// Global rank to report
        #[arg(long, default_value_t = 0)]
        rank: usize,
        /// Transformer layers
        #[arg(long, default_value_t = 32)]
        num_layers: usize,
        /// Attention query heads
        #[arg(long, default_value_t = 32)]
        num_heads: usize,
        /// Key/value heads (GQA)
        #[arg(long, default_value_t = 8)]
        num_kv_heads: usize,
        /// Vocabulary size
        #[arg(long, default_value_t = 32000)]
        vocab: usize,
    },
    /// Warp a logits row through the sampler pipeline (temp/top-k/top-p/min-p)
    Warp {
        /// Comma-separated logits row, e.g. "1.0,2.0,3.0,4.0"
        #[arg(long)]
        logits: String,
        /// Softmax temperature (0 = greedy)
        #[arg(long, default_value_t = 1.0)]
        temperature: f32,
        /// Top-k (-1 disables)
        #[arg(long, default_value_t = -1)]
        top_k: i32,
        /// Nucleus top_p
        #[arg(long, default_value_t = 1.0)]
        top_p: f32,
        /// Min-p relative cutoff
        #[arg(long, default_value_t = 0.0)]
        min_p: f32,
    },
    /// Plan chunked prefill: split prompts across steps under a token budget
    ChunkedPrefill {
        /// Per-step token budget (max_num_batched_tokens)
        #[arg(long, default_value_t = 512)]
        budget: usize,
        /// Comma-separated prompt lengths, e.g. "100,30,800"
        #[arg(long)]
        prompts: String,
    },
    /// Run typical-acceptance speculative decoding over a peaked target row
    SpecTypical {
        /// Number of speculative tokens k
        #[arg(long, default_value_t = 3)]
        k: usize,
        /// Probability mass on the peak token of each position (rest uniform)
        #[arg(long, default_value_t = 0.9)]
        peak: f32,
        /// Vocabulary size of the synthetic target distribution
        #[arg(long, default_value_t = 8)]
        vocab: usize,
    },
}

/// A deterministic stub model that emits a constant token id for every
/// scheduled sequence — exercises the engine control flow without weights.
struct ConstModel(u32);
impl StepModel for ConstModel {
    fn step(&mut self, batch: &[SeqView<'_>]) -> Vec<u32> {
        vec![self.0; batch.len()]
    }
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
        VllmCmd::Engine {
            prompt_tokens,
            max_tokens,
            gpu_blocks,
            block_size,
        } => {
            let cfg = EngineConfig {
                max_num_batched_tokens: 8192,
                max_num_seqs: 256,
                block_size,
                num_gpu_blocks: gpu_blocks,
                eos_token_id: 0,
            };
            let mut engine = LLMEngine::new(cfg, ConstModel(7));
            let params = SamplingParams {
                max_tokens: Some(max_tokens),
                ..Default::default()
            };
            engine
                .try_add_request(1, vec![1; prompt_tokens], params)
                .map_err(|e| anyhow::anyhow!("{e}"))?;
            let mut generated = 0usize;
            let mut reason = None;
            let mut steps = 0usize;
            while engine.has_unfinished_requests() {
                for o in engine.step() {
                    generated += o.new_token_ids.len();
                    if o.finished {
                        reason = o.finish_reason;
                    }
                }
                steps += 1;
            }
            let reason = match reason {
                Some(FinishReason::Stop) => "stop",
                Some(FinishReason::Length) => "length",
                None => "—",
            };
            println!("LLMEngine continuous-batching run");
            println!("  prompt:        {prompt_tokens} tokens");
            println!("  generated:     {generated} tokens over {steps} steps");
            println!("  finish reason: {reason}");
            Ok(())
        }
        VllmCmd::Prefix {
            blocks,
            block_size,
            sequences,
            prefix_blocks,
        } => {
            let mut alloc = PrefixCachingAllocator::new(blocks, block_size);
            for _ in 0..sequences {
                let mut parent = None;
                for b in 0..prefix_blocks {
                    // Same token content across sequences ⇒ shared prefix.
                    let toks: Vec<u32> = (0..block_size).map(|i| (b * block_size + i) as u32).collect();
                    let a = alloc
                        .allocate_immutable(parent, &toks)
                        .map_err(|e| anyhow::anyhow!("{e}"))?;
                    parent = Some(a.content_hash);
                }
            }
            println!("Automatic prefix caching ({sequences} seqs × {prefix_blocks} blocks)");
            println!("  pool:          {blocks} blocks × {block_size} tokens");
            println!("  queries:       {}", alloc.cache_queries());
            println!("  hits:          {}", alloc.cache_hits());
            println!("  cached blocks: {}", alloc.num_cached_blocks());
            println!("  hit rate:      {:.1}%", alloc.hit_rate() * 100.0);
            Ok(())
        }
        VllmCmd::Parallel {
            tp,
            pp,
            rank,
            num_layers,
            num_heads,
            num_kv_heads,
            vocab,
        } => {
            let cfg = ParallelConfig {
                tensor_parallel_size: tp,
                pipeline_parallel_size: pp,
            };
            if rank >= cfg.world_size() {
                return Err(anyhow::anyhow!(
                    "rank {rank} out of range for world_size {}",
                    cfg.world_size()
                ));
            }
            let (tp_rank, pp_rank) = (cfg.tp_rank(rank), cfg.pp_rank(rank));
            let vs = vocab_partition(vocab, tp_rank, tp);
            let (start_layer, end_layer) = get_pp_indices(num_layers, pp_rank, pp);
            println!("Tensor/pipeline-parallel plan (tp={tp}, pp={pp}, rank={rank})");
            println!("  world size:    {}", cfg.world_size());
            println!("  tp_rank:       {tp_rank}  (group {:?})", cfg.tp_group(rank));
            println!("  pp_rank:       {pp_rank}  (group {:?})", cfg.pp_group(rank));
            println!(
                "  q heads/rank:  {}",
                attn_heads_per_rank(num_heads, tp).map_err(|e| anyhow::anyhow!("{e}"))?
            );
            println!("  kv heads/rank: {}", kv_heads_per_rank(num_kv_heads, tp));
            println!(
                "  vocab shard:   [{}, {}) of padded {} ({} rows)",
                vs.start, vs.end, vs.padded_vocab, vs.num_embeddings_per_partition
            );
            println!("  layers:        [{start_layer}, {end_layer}) of {num_layers}");
            Ok(())
        }
        VllmCmd::Warp {
            logits,
            temperature,
            top_k,
            top_p,
            min_p,
        } => {
            let mut row: Vec<f32> = logits
                .split(',')
                .map(|s| s.trim().parse::<f32>())
                .collect::<Result<_, _>>()
                .map_err(|e| anyhow::anyhow!("bad logits list: {e}"))?;
            let params = SamplingParams {
                temperature,
                top_k,
                top_p,
                min_p,
                ..Default::default()
            };
            let probs = vllm_sampler::process(&mut row, &params, &[], &[]);
            println!("Logits-sampler warp (temp={temperature}, top_k={top_k}, top_p={top_p}, min_p={min_p})");
            for (i, (l, p)) in row.iter().zip(probs.iter()).enumerate() {
                let l = if l.is_finite() {
                    format!("{l:.4}")
                } else {
                    "-inf".to_string()
                };
                println!("  token {i:>3}: logit {l:>10}   prob {:.4}", p);
            }
            Ok(())
        }
        VllmCmd::ChunkedPrefill { budget, prompts } => {
            let lengths: Vec<usize> = prompts
                .split(',')
                .map(|s| s.trim().parse::<usize>())
                .collect::<Result<_, _>>()
                .map_err(|e| anyhow::anyhow!("bad prompt list: {e}"))?;
            let mut planner = ChunkedPrefillPlanner::new(budget);
            for (i, len) in lengths.iter().enumerate() {
                planner.add(i as u64, *len);
            }
            println!("Chunked-prefill plan (budget={budget}/step, {} prompts)", lengths.len());
            let mut step = 0usize;
            while !planner.is_empty() {
                let chunks = planner.step();
                let rendered: Vec<String> = chunks
                    .iter()
                    .map(|c| {
                        format!("seq{}:{}{}", c.id, c.tokens, if c.done { "*" } else { "" })
                    })
                    .collect();
                println!("  step {step:>2}: [{}]", rendered.join(", "));
                step += 1;
            }
            println!("  ({step} steps; '*' = prompt fully prefilled)");
            Ok(())
        }
        VllmCmd::SpecTypical { k, peak, vocab } => {
            if vocab < 2 {
                return Err(anyhow::anyhow!("vocab must be >= 2"));
            }
            // Synthetic peaked target rows: peak on token 0, rest uniform.
            let rest = (1.0 - peak) / (vocab - 1) as f32;
            let row: Vec<f32> = (0..vocab)
                .map(|j| if j == 0 { peak } else { rest })
                .collect();
            let target: Vec<Vec<f32>> = (0..=k).map(|_| row.clone()).collect();
            // Draft all peaks (token 0) -> all should be typical and accepted.
            let draft: Vec<u32> = vec![0; k];
            let sampler = TypicalAcceptanceSampler::with_defaults(k);
            let r = sampler.sample(&draft, &target);
            println!("Typical-acceptance spec decode (k={k}, peak={peak}, vocab={vocab})");
            println!("  threshold:     {:.4}", sampler.posterior_threshold());
            println!("  alpha:         {:.4}", sampler.posterior_alpha());
            println!("  accepted:      {} / {k}", r.accepted);
            println!("  emitted:       {:?}", r.emitted);
            println!("  bonus token:   {}", if r.all_accepted { "yes" } else { "no (rejected)" });
            Ok(())
        }
    }
}
