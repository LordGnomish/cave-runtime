//! cave-local-llm CLI — offline draft generator for Cave Runtime.
//!
//! Usage:
//!   cave-local-llm run --crate cave-secrets [--dry-run] [--model <model>]

use anyhow::{Context, Result};
use cave_local_llm::{
    draft::{
        build_ollama_prompt, draft_filename, parse_ollama_response, Draft, DraftFrontmatter,
        DraftStatus,
    },
    manifest::{find_missing_functions, read_crate_manifest},
    metrics::DraftMetrics,
    ollama::{GenerateRequest, OllamaClient},
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
        Commands::Run { crate_name, workspace_root, model, dry_run } => {
            let workspace_root = workspace_root
                .canonicalize()
                .with_context(|| format!("workspace root '{}' not found", workspace_root.display()))?;

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

            let upstream_repo =
                format!("{}/{}", manifest.upstream.org, manifest.upstream.repo);

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
            let draft = Draft { frontmatter: fm, failing_test: test_code, skeleton: skel_code };
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
            println!("  function: {} → {}", target.upstream_name, target.local_name);
            println!("  model:    {model}");
            println!("  elapsed:  {elapsed:.2}s");

            Ok(())
        }
    }
}
