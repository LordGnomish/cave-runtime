// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Source registries — HuggingFace, Ollama library, LMSys leaderboard, and
//! GitHub backend releases (vLLM / llama.cpp / MLX-LM).
//!
//! Each source produces a normalised [`Candidate`]. A seed catalog ships
//! in-binary so that `--mode report` always emits a useful list even when
//! the network is unreachable (CI, airplane mode, sandbox runs).

use serde::{Deserialize, Serialize};

use crate::error::TrackerResult;

/// Where a candidate model entry came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SourceKind {
    HuggingFace,
    OllamaLibrary,
    LmsysLeaderboard,
    GithubBackend,
    SeedCatalog,
}

impl SourceKind {
    pub fn slug(&self) -> &'static str {
        match self {
            Self::HuggingFace => "huggingface",
            Self::OllamaLibrary => "ollama_library",
            Self::LmsysLeaderboard => "lmsys_leaderboard",
            Self::GithubBackend => "github_backend",
            Self::SeedCatalog => "seed_catalog",
        }
    }
}

/// A model entry surfaced by one of the [`SourceKind`] sources, in the
/// minimal shape needed for selection.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Candidate {
    pub source: SourceKind,
    pub model_id: String,
    pub family: String,
    pub license: String,
    /// Estimated VRAM footprint at the dominant quant we'd serve, GiB.
    pub vram_gib: f32,
    /// Estimated on-disk size at the dominant quant, GiB.
    pub disk_gib: f32,
    /// Free-form quant tag (`Q4_K_M`, `mxfp8`, `bf16`, ...). Empty if N/A.
    pub quant: String,
    /// Upstream-reported reference (HF model id, Ollama tag, LMSys
    /// arena slug, GitHub release tag). Pinned so the daily report is
    /// auditable.
    pub upstream_ref: String,
    /// LMSys-style relative score, if known. Higher is better.
    /// `None` for sources that do not publish a score.
    pub score_hint: Option<f32>,
}

/// Endpoints + UA pins for the live (network) source fetchers.
#[derive(Debug, Clone)]
pub struct RegistryEndpoints {
    pub huggingface_api: String,
    pub ollama_library_index: String,
    pub lmsys_leaderboard_csv: String,
    pub github_api: String,
    pub user_agent: String,
}

impl RegistryEndpoints {
    pub fn defaults() -> Self {
        Self {
            // Public model index — `?sort=trending&direction=-1&limit=...`.
            huggingface_api: "https://huggingface.co/api/models".to_string(),
            // Ollama serves an HTML index; we treat it as a probe-only
            // signal in live mode and rely on the seed catalog for the
            // hot family list. Phase 1 will parse the index page.
            ollama_library_index: "https://ollama.com/library".to_string(),
            // Mirror of the public LMSys leaderboard CSV. Switched to
            // their published JSON when stable.
            lmsys_leaderboard_csv:
                "https://huggingface.co/spaces/lmsys/chatbot-arena-leaderboard/raw/main/elo_results.csv"
                    .to_string(),
            github_api: "https://api.github.com".to_string(),
            user_agent: "cave-llm-tracker/0.1 (+https://github.com/cave-runtime)".to_string(),
        }
    }
}

/// In-binary seed catalog — covers the working set of local-LLM models
/// that we want the daily report to *always* surface, regardless of
/// upstream availability. This is the floor; live sources extend the
/// list, never shrink it.
pub fn seed_catalog() -> Vec<Candidate> {
    vec![
        Candidate {
            source: SourceKind::SeedCatalog,
            model_id: "qwen3.6:35b-a3b-coding-mxfp8".to_string(),
            family: "qwen3.6".to_string(),
            license: "Apache-2.0".to_string(),
            vram_gib: 22.0,
            disk_gib: 24.0,
            quant: "mxfp8".to_string(),
            upstream_ref: "Qwen/Qwen3.6-35B-A3B-Coding".to_string(),
            score_hint: Some(1241.0),
        },
        Candidate {
            source: SourceKind::SeedCatalog,
            model_id: "qwen3.6:72b-instruct-mxfp8".to_string(),
            family: "qwen3.6".to_string(),
            license: "Apache-2.0".to_string(),
            vram_gib: 48.0,
            disk_gib: 52.0,
            quant: "mxfp8".to_string(),
            upstream_ref: "Qwen/Qwen3.6-72B-Instruct".to_string(),
            score_hint: Some(1278.0),
        },
        Candidate {
            source: SourceKind::SeedCatalog,
            model_id: "llama4:70b-instruct-Q4_K_M".to_string(),
            family: "llama4".to_string(),
            license: "Llama-4-Community".to_string(),
            vram_gib: 40.0,
            disk_gib: 44.0,
            quant: "Q4_K_M".to_string(),
            upstream_ref: "meta-llama/Llama-4-70B-Instruct".to_string(),
            score_hint: Some(1252.0),
        },
        Candidate {
            source: SourceKind::SeedCatalog,
            model_id: "deepseek-coder-v3:33b-Q5_K_M".to_string(),
            family: "deepseek-coder-v3".to_string(),
            license: "DeepSeek".to_string(),
            vram_gib: 28.0,
            disk_gib: 30.0,
            quant: "Q5_K_M".to_string(),
            upstream_ref: "deepseek-ai/DeepSeek-Coder-V3-33B".to_string(),
            score_hint: Some(1231.0),
        },
        Candidate {
            source: SourceKind::SeedCatalog,
            model_id: "mistral-large-3:24b-Q4_K_M".to_string(),
            family: "mistral-large-3".to_string(),
            license: "Apache-2.0".to_string(),
            vram_gib: 18.0,
            disk_gib: 20.0,
            quant: "Q4_K_M".to_string(),
            upstream_ref: "mistralai/Mistral-Large-3-24B".to_string(),
            score_hint: Some(1224.0),
        },
        Candidate {
            source: SourceKind::SeedCatalog,
            model_id: "phi-5:14b-mlx-bf16".to_string(),
            family: "phi-5".to_string(),
            license: "MIT".to_string(),
            vram_gib: 16.0,
            disk_gib: 18.0,
            quant: "bf16".to_string(),
            upstream_ref: "microsoft/Phi-5-14B".to_string(),
            score_hint: Some(1198.0),
        },
        Candidate {
            source: SourceKind::SeedCatalog,
            model_id: "gemma3:27b-Q5_K_M".to_string(),
            family: "gemma3".to_string(),
            license: "Gemma".to_string(),
            vram_gib: 20.0,
            disk_gib: 22.0,
            quant: "Q5_K_M".to_string(),
            upstream_ref: "google/gemma-3-27b".to_string(),
            score_hint: Some(1219.0),
        },
        Candidate {
            source: SourceKind::SeedCatalog,
            model_id: "yi-coder-2:34b-Q4_K_M".to_string(),
            family: "yi-coder-2".to_string(),
            license: "Apache-2.0".to_string(),
            vram_gib: 26.0,
            disk_gib: 28.0,
            quant: "Q4_K_M".to_string(),
            upstream_ref: "01-ai/Yi-Coder-2-34B".to_string(),
            score_hint: Some(1208.0),
        },
    ]
}

/// Async fetchers for the four live sources. Each returns an empty
/// vector instead of failing on transport errors — that way the daily
/// report degrades gracefully when the network is down (the seed
/// catalog still produces a useful row set).
pub struct LiveFetcher {
    pub endpoints: RegistryEndpoints,
    pub client: reqwest::Client,
}

impl LiveFetcher {
    pub fn new() -> Self {
        let endpoints = RegistryEndpoints::defaults();
        let client = reqwest::Client::builder()
            .user_agent(endpoints.user_agent.clone())
            .timeout(std::time::Duration::from_secs(20))
            .build()
            .expect("reqwest client");
        Self { endpoints, client }
    }

    pub async fn fetch_huggingface(&self, limit: usize) -> TrackerResult<Vec<Candidate>> {
        let url = format!(
            "{}?search=instruct&sort=downloads&direction=-1&limit={}&full=false",
            self.endpoints.huggingface_api, limit
        );
        let resp = match self.client.get(&url).send().await {
            Ok(r) => r,
            Err(_) => return Ok(Vec::new()),
        };
        let body: serde_json::Value = match resp.json().await {
            Ok(v) => v,
            Err(_) => return Ok(Vec::new()),
        };
        Ok(parse_huggingface(&body))
    }

    pub async fn fetch_lmsys(&self) -> TrackerResult<Vec<Candidate>> {
        let resp = match self
            .client
            .get(&self.endpoints.lmsys_leaderboard_csv)
            .send()
            .await
        {
            Ok(r) => r,
            Err(_) => return Ok(Vec::new()),
        };
        let body = match resp.text().await {
            Ok(t) => t,
            Err(_) => return Ok(Vec::new()),
        };
        Ok(parse_lmsys_csv(&body))
    }

    pub async fn fetch_ollama_library(&self) -> TrackerResult<Vec<Candidate>> {
        let resp = match self
            .client
            .get(&self.endpoints.ollama_library_index)
            .send()
            .await
        {
            Ok(r) => r,
            Err(_) => return Ok(Vec::new()),
        };
        let body = match resp.text().await {
            Ok(t) => t,
            Err(_) => return Ok(Vec::new()),
        };
        Ok(parse_ollama_library(&body))
    }

    pub async fn fetch_github_backend(&self, repos: &[(&str, &str)]) -> TrackerResult<Vec<Candidate>> {
        let mut out = Vec::new();
        for (org, repo) in repos {
            let url = format!(
                "{}/repos/{}/{}/releases/latest",
                self.endpoints.github_api, org, repo
            );
            let resp = match self.client.get(&url).send().await {
                Ok(r) => r,
                Err(_) => continue,
            };
            let body: serde_json::Value = match resp.json().await {
                Ok(v) => v,
                Err(_) => continue,
            };
            if let Some(tag) = body.get("tag_name").and_then(|v| v.as_str()) {
                out.push(Candidate {
                    source: SourceKind::GithubBackend,
                    model_id: format!("{}/{}:{}", org, repo, tag),
                    family: (*repo).to_string(),
                    license: license_for_backend(repo).to_string(),
                    vram_gib: 0.0,
                    disk_gib: 0.0,
                    quant: String::new(),
                    upstream_ref: format!("{}/{}@{}", org, repo, tag),
                    score_hint: None,
                });
            }
        }
        Ok(out)
    }
}

fn license_for_backend(repo: &str) -> &'static str {
    match repo {
        "vllm" => "Apache-2.0",
        "llama.cpp" => "MIT",
        "mlx-lm" | "mlx-examples" => "MIT",
        _ => "unknown",
    }
}

pub(crate) fn parse_huggingface(v: &serde_json::Value) -> Vec<Candidate> {
    let Some(arr) = v.as_array() else {
        return Vec::new();
    };
    arr.iter()
        .filter_map(|item| {
            let id = item.get("id")?.as_str()?;
            let license = item
                .get("tags")
                .and_then(|t| t.as_array())
                .and_then(|tags| {
                    tags.iter()
                        .filter_map(|t| t.as_str())
                        .find_map(|t| t.strip_prefix("license:").map(str::to_owned))
                })
                .unwrap_or_else(|| "unknown".to_string());
            Some(Candidate {
                source: SourceKind::HuggingFace,
                model_id: id.to_string(),
                family: id.split('/').next_back().unwrap_or(id).to_string(),
                license,
                vram_gib: 0.0,
                disk_gib: 0.0,
                quant: String::new(),
                upstream_ref: id.to_string(),
                score_hint: None,
            })
        })
        .collect()
}

pub(crate) fn parse_lmsys_csv(text: &str) -> Vec<Candidate> {
    let mut out = Vec::new();
    let mut lines = text.lines();
    let Some(header) = lines.next() else {
        return out;
    };
    let cols: Vec<&str> = header.split(',').map(str::trim).collect();
    let model_idx = cols.iter().position(|c| c.eq_ignore_ascii_case("model"));
    let score_idx = cols
        .iter()
        .position(|c| c.eq_ignore_ascii_case("elo") || c.eq_ignore_ascii_case("rating"));
    for line in lines {
        let fields: Vec<&str> = line.split(',').map(str::trim).collect();
        let Some(mi) = model_idx else {
            break;
        };
        if fields.len() <= mi {
            continue;
        }
        let model = fields[mi].trim_matches('"');
        if model.is_empty() {
            continue;
        }
        let score = score_idx
            .and_then(|si| fields.get(si))
            .and_then(|s| s.parse::<f32>().ok());
        out.push(Candidate {
            source: SourceKind::LmsysLeaderboard,
            model_id: model.to_string(),
            family: model.to_string(),
            license: "unknown".to_string(),
            vram_gib: 0.0,
            disk_gib: 0.0,
            quant: String::new(),
            upstream_ref: format!("lmsys:{model}"),
            score_hint: score,
        });
    }
    out
}

pub(crate) fn parse_ollama_library(html: &str) -> Vec<Candidate> {
    let mut out = Vec::new();
    for line in html.lines() {
        let trimmed = line.trim();
        if let Some(rest) = trimmed.strip_prefix("<a href=\"/library/")
            && let Some(end) = rest.find('"')
        {
            let slug = &rest[..end];
            if slug.is_empty() || slug.contains('/') {
                continue;
            }
            out.push(Candidate {
                source: SourceKind::OllamaLibrary,
                model_id: format!("{slug}:latest"),
                family: slug.to_string(),
                license: "unknown".to_string(),
                vram_gib: 0.0,
                disk_gib: 0.0,
                quant: String::new(),
                upstream_ref: format!("ollama:{slug}"),
                score_hint: None,
            });
        }
    }
    out
}

/// The canonical list of "backend release" sources we care about — the
/// runtimes that actually load the models onto Burak's box.
pub fn default_backend_repos() -> Vec<(&'static str, &'static str)> {
    vec![
        ("vllm-project", "vllm"),
        ("ggml-org", "llama.cpp"),
        ("ml-explore", "mlx-lm"),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seed_catalog_has_at_least_five_candidates() {
        let s = seed_catalog();
        assert!(s.len() >= 5, "seed catalog must have >= 5 entries; got {}", s.len());
        assert!(s.iter().any(|c| c.model_id.starts_with("qwen3.6:35b")));
    }

    #[test]
    fn source_slugs_are_stable() {
        assert_eq!(SourceKind::HuggingFace.slug(), "huggingface");
        assert_eq!(SourceKind::OllamaLibrary.slug(), "ollama_library");
        assert_eq!(SourceKind::LmsysLeaderboard.slug(), "lmsys_leaderboard");
        assert_eq!(SourceKind::GithubBackend.slug(), "github_backend");
        assert_eq!(SourceKind::SeedCatalog.slug(), "seed_catalog");
    }

    #[test]
    fn parse_hf_extracts_id_and_license_tag() {
        let v: serde_json::Value = serde_json::from_str(
            r#"[{"id":"meta-llama/Llama-4-70B-Instruct","tags":["license:llama-4","instruct"]}]"#,
        )
        .unwrap();
        let cs = parse_huggingface(&v);
        assert_eq!(cs.len(), 1);
        assert_eq!(cs[0].license, "llama-4");
        assert_eq!(cs[0].source, SourceKind::HuggingFace);
    }

    #[test]
    fn parse_lmsys_csv_reads_model_and_score() {
        let csv = "Model,Elo\nqwen3.6-72b,1278.0\nllama4-70b,1252.0\n";
        let cs = parse_lmsys_csv(csv);
        assert_eq!(cs.len(), 2);
        assert_eq!(cs[0].model_id, "qwen3.6-72b");
        assert_eq!(cs[0].score_hint, Some(1278.0));
        assert_eq!(cs[0].source, SourceKind::LmsysLeaderboard);
    }

    #[test]
    fn parse_ollama_library_picks_up_anchors() {
        let html = r#"
            <a href="/library/qwen3.6">qwen3.6</a>
            <a href="/library/llama4">llama4</a>
            <a href="/library/qwen3.6/tags">don't pick subpaths</a>
        "#;
        let cs = parse_ollama_library(html);
        let names: Vec<_> = cs.iter().map(|c| c.family.as_str()).collect();
        assert!(names.contains(&"qwen3.6"));
        assert!(names.contains(&"llama4"));
        for c in &cs {
            assert_eq!(c.source, SourceKind::OllamaLibrary);
        }
    }

    #[test]
    fn default_backend_repos_covers_three_runtimes() {
        let r = default_backend_repos();
        assert!(r.iter().any(|(_, n)| *n == "vllm"));
        assert!(r.iter().any(|(_, n)| *n == "llama.cpp"));
        assert!(r.iter().any(|(_, n)| *n == "mlx-lm"));
    }
}
