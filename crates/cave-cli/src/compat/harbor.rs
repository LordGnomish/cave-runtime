// SPDX-License-Identifier: AGPL-3.0-or-later
//! `cavectl harbor …` — Harbor (and OCI registry) compat shim.

use anyhow::{bail, Result};
use clap::Subcommand;
use serde_json::{json, Value};

use crate::native::{HttpVerb, PreparedRequest};

#[derive(Subcommand, Debug, Clone)]
pub enum HarborCmd {
    /// Authenticate against the registry.
    Login {
        registry: String,
        #[arg(long)]
        username: String,
    },
    Logout {
        registry: String,
    },
    /// Push an image to the registry.
    Push {
        image: String,
        #[arg(long)]
        sign: bool,
    },
    /// Pull an image from the registry.
    Pull {
        image: String,
    },
    /// Tag a remote image.
    Tag {
        src: String,
        dst: String,
    },
    Repo {
        #[command(subcommand)]
        cmd: RepoCmd,
    },
    Project {
        #[command(subcommand)]
        cmd: ProjectCmd,
    },
    Replication {
        #[command(subcommand)]
        cmd: ReplicationCmd,
    },
    Scan {
        image: String,
    },
    /// Run garbage collection.
    Gc {
        #[arg(long)]
        dry_run: bool,
    },
}

#[derive(Subcommand, Debug, Clone)]
pub enum RepoCmd {
    /// List repos in a project.
    List {
        project: String,
    },
    Tags {
        repo: String,
    },
    /// Delete a repo or specific tag.
    Delete {
        target: String,
    },
}

#[derive(Subcommand, Debug, Clone)]
pub enum ProjectCmd {
    Create {
        name: String,
        #[arg(long, default_value = "private")]
        visibility: String,
    },
    Delete {
        name: String,
    },
    List,
    Get {
        name: String,
    },
}

#[derive(Subcommand, Debug, Clone)]
pub enum ReplicationCmd {
    Create {
        name: String,
        #[arg(long)]
        src: String,
        #[arg(long)]
        dst: String,
    },
    Trigger {
        id: String,
    },
    List,
    Delete {
        id: String,
    },
}

const VISIBILITIES: &[&str] = &["public", "private"];

pub fn prepare(cmd: &HarborCmd) -> Result<PreparedRequest> {
    match cmd {
        HarborCmd::Login { registry, username } => {
            if registry.is_empty() || username.is_empty() {
                bail!("registry and username required");
            }
            Ok(PreparedRequest::new(
                HttpVerb::Post,
                "/api/compat/harbor/v2/auth/login",
            )
            .with_body(json!({"registry": registry, "username": username})))
        }
        HarborCmd::Logout { registry } => Ok(PreparedRequest::new(
            HttpVerb::Post,
            "/api/compat/harbor/v2/auth/logout",
        )
        .with_body(json!({"registry": registry}))),
        HarborCmd::Push { image, sign } => {
            let (project, repo, tag) = parse_image(image)?;
            let mut body: Value = json!({
                "project": project,
                "repository": repo,
                "tag": tag,
            });
            if *sign {
                body["sign"] = json!(true);
            }
            Ok(PreparedRequest::new(
                HttpVerb::Post,
                format!(
                    "/api/compat/harbor/v2/projects/{}/repositories/{}/artifacts",
                    project, repo
                ),
            )
            .with_body(body))
        }
        HarborCmd::Pull { image } => {
            let (project, repo, tag) = parse_image(image)?;
            Ok(PreparedRequest::new(
                HttpVerb::Get,
                format!(
                    "/api/compat/harbor/v2/projects/{}/repositories/{}/artifacts/{}",
                    project, repo, tag
                ),
            ))
        }
        HarborCmd::Tag { src, dst } => {
            let (sp, sr, st) = parse_image(src)?;
            let (dp, dr, dt) = parse_image(dst)?;
            Ok(PreparedRequest::new(
                HttpVerb::Post,
                format!(
                    "/api/compat/harbor/v2/projects/{}/repositories/{}/artifacts/{}/tags",
                    sp, sr, st
                ),
            )
            .with_body(json!({
                "dst_project": dp,
                "dst_repository": dr,
                "dst_tag": dt,
            })))
        }
        HarborCmd::Repo { cmd } => prepare_repo(cmd),
        HarborCmd::Project { cmd } => prepare_project(cmd),
        HarborCmd::Replication { cmd } => prepare_replication(cmd),
        HarborCmd::Scan { image } => {
            let (project, repo, tag) = parse_image(image)?;
            Ok(PreparedRequest::new(
                HttpVerb::Post,
                format!(
                    "/api/compat/harbor/v2/projects/{}/repositories/{}/artifacts/{}/scan",
                    project, repo, tag
                ),
            )
            .with_body(json!({})))
        }
        HarborCmd::Gc { dry_run } => Ok(PreparedRequest::new(
            HttpVerb::Post,
            "/api/compat/harbor/v2/system/gc",
        )
        .with_body(json!({"dry_run": dry_run}))),
    }
}

fn prepare_repo(cmd: &RepoCmd) -> Result<PreparedRequest> {
    match cmd {
        RepoCmd::List { project } => Ok(PreparedRequest::new(
            HttpVerb::Get,
            format!("/api/compat/harbor/v2/projects/{}/repositories", project),
        )),
        RepoCmd::Tags { repo } => {
            let parts: Vec<&str> = repo.splitn(2, '/').collect();
            if parts.len() != 2 {
                bail!("repo must be `<project>/<repo>`");
            }
            Ok(PreparedRequest::new(
                HttpVerb::Get,
                format!(
                    "/api/compat/harbor/v2/projects/{}/repositories/{}/artifacts",
                    parts[0], parts[1]
                ),
            ))
        }
        RepoCmd::Delete { target } => {
            // target either "<project>/<repo>" or "<project>/<repo>:<tag>"
            let (project, rest) = target
                .split_once('/')
                .ok_or_else(|| anyhow::anyhow!("target must include project"))?;
            let path = match rest.split_once(':') {
                Some((repo, tag)) => format!(
                    "/api/compat/harbor/v2/projects/{}/repositories/{}/artifacts/{}",
                    project, repo, tag
                ),
                None => format!(
                    "/api/compat/harbor/v2/projects/{}/repositories/{}",
                    project, rest
                ),
            };
            Ok(PreparedRequest::new(HttpVerb::Delete, path))
        }
    }
}

fn prepare_project(cmd: &ProjectCmd) -> Result<PreparedRequest> {
    match cmd {
        ProjectCmd::Create { name, visibility } => {
            if !VISIBILITIES.contains(&visibility.as_str()) {
                bail!(
                    "unknown visibility `{}`; want one of {:?}",
                    visibility,
                    VISIBILITIES
                );
            }
            Ok(
                PreparedRequest::new(HttpVerb::Post, "/api/compat/harbor/v2/projects").with_body(
                    json!({"project_name": name, "metadata": {"public": visibility == "public"}}),
                ),
            )
        }
        ProjectCmd::Delete { name } => Ok(PreparedRequest::new(
            HttpVerb::Delete,
            format!("/api/compat/harbor/v2/projects/{}", name),
        )),
        ProjectCmd::List => Ok(PreparedRequest::new(
            HttpVerb::Get,
            "/api/compat/harbor/v2/projects",
        )),
        ProjectCmd::Get { name } => Ok(PreparedRequest::new(
            HttpVerb::Get,
            format!("/api/compat/harbor/v2/projects/{}", name),
        )),
    }
}

fn prepare_replication(cmd: &ReplicationCmd) -> Result<PreparedRequest> {
    match cmd {
        ReplicationCmd::Create { name, src, dst } => Ok(PreparedRequest::new(
            HttpVerb::Post,
            "/api/compat/harbor/v2/replication/policies",
        )
        .with_body(json!({"name": name, "src_registry": src, "dst_registry": dst}))),
        ReplicationCmd::Trigger { id } => Ok(PreparedRequest::new(
            HttpVerb::Post,
            format!("/api/compat/harbor/v2/replication/executions/{}", id),
        )
        .with_body(json!({}))),
        ReplicationCmd::List => Ok(PreparedRequest::new(
            HttpVerb::Get,
            "/api/compat/harbor/v2/replication/policies",
        )),
        ReplicationCmd::Delete { id } => Ok(PreparedRequest::new(
            HttpVerb::Delete,
            format!("/api/compat/harbor/v2/replication/policies/{}", id),
        )),
    }
}

/// Parse a Docker-style image ref into (project, repo, tag).
///
/// Forms: `<project>/<repo>` (tag defaults to `latest`),
/// `<project>/<repo>:<tag>`. Registry prefix is dropped.
pub fn parse_image(image: &str) -> Result<(String, String, String)> {
    let mut s = image;
    // Drop registry/host prefix if present (contains a '.' or ':')
    if let Some(idx) = s.find('/') {
        let head = &s[..idx];
        if head.contains('.') || head.contains(':') {
            s = &s[idx + 1..];
        }
    }
    let (project, rest) = s
        .split_once('/')
        .ok_or_else(|| anyhow::anyhow!("image must be `<project>/<repo>[:<tag>]`"))?;
    let (repo, tag) = match rest.split_once(':') {
        Some((r, t)) => (r.to_string(), t.to_string()),
        None => (rest.to_string(), "latest".to_string()),
    };
    if project.is_empty() || repo.is_empty() {
        bail!("image must be `<project>/<repo>`");
    }
    Ok((project.to_string(), repo, tag))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn login() {
        let r = prepare(&HarborCmd::Login {
            registry: "harbor.example.com".into(),
            username: "alice".into(),
        })
        .unwrap();
        assert_eq!(r.verb, HttpVerb::Post);
        assert_eq!(r.body.unwrap()["registry"], "harbor.example.com");
    }

    #[test]
    fn login_rejects_empty_registry() {
        assert!(prepare(&HarborCmd::Login {
            registry: "".into(),
            username: "x".into(),
        })
        .is_err());
    }

    #[test]
    fn logout() {
        let r = prepare(&HarborCmd::Logout {
            registry: "harbor.example.com".into(),
        })
        .unwrap();
        assert!(r.path.ends_with("/auth/logout"));
    }

    #[test]
    fn push_basic() {
        let r = prepare(&HarborCmd::Push {
            image: "library/nginx:1.27".into(),
            sign: false,
        })
        .unwrap();
        assert!(r.path.contains("/projects/library/repositories/nginx/"));
        let body = r.body.unwrap();
        assert_eq!(body["tag"], "1.27");
    }

    #[test]
    fn push_default_latest() {
        let r = prepare(&HarborCmd::Push {
            image: "library/nginx".into(),
            sign: false,
        })
        .unwrap();
        assert_eq!(r.body.unwrap()["tag"], "latest");
    }

    #[test]
    fn push_sign() {
        let r = prepare(&HarborCmd::Push {
            image: "library/nginx".into(),
            sign: true,
        })
        .unwrap();
        assert_eq!(r.body.unwrap()["sign"], true);
    }

    #[test]
    fn pull_with_tag() {
        let r = prepare(&HarborCmd::Pull {
            image: "library/nginx:1.27".into(),
        })
        .unwrap();
        assert!(r.path.ends_with("/artifacts/1.27"));
    }

    #[test]
    fn pull_default_latest_in_path() {
        let r = prepare(&HarborCmd::Pull {
            image: "library/nginx".into(),
        })
        .unwrap();
        assert!(r.path.ends_with("/artifacts/latest"));
    }

    #[test]
    fn tag_image() {
        let r = prepare(&HarborCmd::Tag {
            src: "library/nginx:1.27".into(),
            dst: "library/nginx:stable".into(),
        })
        .unwrap();
        let body = r.body.unwrap();
        assert_eq!(body["dst_tag"], "stable");
    }

    #[test]
    fn parse_image_no_tag() {
        let (p, r, t) = parse_image("library/nginx").unwrap();
        assert_eq!((p.as_str(), r.as_str(), t.as_str()), ("library", "nginx", "latest"));
    }

    #[test]
    fn parse_image_with_tag() {
        let (_, _, t) = parse_image("library/nginx:1.27").unwrap();
        assert_eq!(t, "1.27");
    }

    #[test]
    fn parse_image_strips_registry() {
        let (p, r, _) = parse_image("harbor.example.com/library/nginx:1.27").unwrap();
        assert_eq!(p, "library");
        assert_eq!(r, "nginx");
    }

    #[test]
    fn parse_image_strips_registry_with_port() {
        let (p, _, _) = parse_image("harbor:5000/library/nginx").unwrap();
        assert_eq!(p, "library");
    }

    #[test]
    fn parse_image_rejects_no_slash() {
        assert!(parse_image("nginx").is_err());
    }

    #[test]
    fn parse_image_rejects_empty_repo() {
        assert!(parse_image("library/").is_err());
    }

    #[test]
    fn repo_list() {
        let r = prepare(&HarborCmd::Repo {
            cmd: RepoCmd::List {
                project: "library".into(),
            },
        })
        .unwrap();
        assert_eq!(r.path, "/api/compat/harbor/v2/projects/library/repositories");
    }

    #[test]
    fn repo_tags() {
        let r = prepare(&HarborCmd::Repo {
            cmd: RepoCmd::Tags {
                repo: "library/nginx".into(),
            },
        })
        .unwrap();
        assert!(r
            .path
            .ends_with("/projects/library/repositories/nginx/artifacts"));
    }

    #[test]
    fn repo_tags_rejects_one_segment() {
        assert!(prepare(&HarborCmd::Repo {
            cmd: RepoCmd::Tags {
                repo: "library".into(),
            },
        })
        .is_err());
    }

    #[test]
    fn repo_delete_repo() {
        let r = prepare(&HarborCmd::Repo {
            cmd: RepoCmd::Delete {
                target: "library/nginx".into(),
            },
        })
        .unwrap();
        assert_eq!(r.verb, HttpVerb::Delete);
        assert_eq!(
            r.path,
            "/api/compat/harbor/v2/projects/library/repositories/nginx"
        );
    }

    #[test]
    fn repo_delete_tag() {
        let r = prepare(&HarborCmd::Repo {
            cmd: RepoCmd::Delete {
                target: "library/nginx:old".into(),
            },
        })
        .unwrap();
        assert!(r
            .path
            .ends_with("/projects/library/repositories/nginx/artifacts/old"));
    }

    #[test]
    fn project_create_visibility_public() {
        let r = prepare(&HarborCmd::Project {
            cmd: ProjectCmd::Create {
                name: "p".into(),
                visibility: "public".into(),
            },
        })
        .unwrap();
        let body = r.body.unwrap();
        assert_eq!(body["metadata"]["public"], true);
    }

    #[test]
    fn project_create_visibility_private() {
        let r = prepare(&HarborCmd::Project {
            cmd: ProjectCmd::Create {
                name: "p".into(),
                visibility: "private".into(),
            },
        })
        .unwrap();
        assert_eq!(r.body.unwrap()["metadata"]["public"], false);
    }

    #[test]
    fn project_create_rejects_unknown_visibility() {
        assert!(prepare(&HarborCmd::Project {
            cmd: ProjectCmd::Create {
                name: "p".into(),
                visibility: "shared".into(),
            },
        })
        .is_err());
    }

    #[test]
    fn project_get_path() {
        let r = prepare(&HarborCmd::Project {
            cmd: ProjectCmd::Get {
                name: "library".into(),
            },
        })
        .unwrap();
        assert_eq!(r.path, "/api/compat/harbor/v2/projects/library");
    }

    #[test]
    fn project_list() {
        let r = prepare(&HarborCmd::Project {
            cmd: ProjectCmd::List,
        })
        .unwrap();
        assert_eq!(r.path, "/api/compat/harbor/v2/projects");
    }

    #[test]
    fn replication_create() {
        let r = prepare(&HarborCmd::Replication {
            cmd: ReplicationCmd::Create {
                name: "p".into(),
                src: "src".into(),
                dst: "dst".into(),
            },
        })
        .unwrap();
        assert_eq!(r.verb, HttpVerb::Post);
        let body = r.body.unwrap();
        assert_eq!(body["src_registry"], "src");
        assert_eq!(body["dst_registry"], "dst");
    }

    #[test]
    fn replication_trigger() {
        let r = prepare(&HarborCmd::Replication {
            cmd: ReplicationCmd::Trigger {
                id: "x".into(),
            },
        })
        .unwrap();
        assert!(r.path.ends_with("/replication/executions/x"));
    }

    #[test]
    fn replication_list() {
        let r = prepare(&HarborCmd::Replication {
            cmd: ReplicationCmd::List,
        })
        .unwrap();
        assert_eq!(r.path, "/api/compat/harbor/v2/replication/policies");
    }

    #[test]
    fn replication_delete() {
        let r = prepare(&HarborCmd::Replication {
            cmd: ReplicationCmd::Delete { id: "x".into() },
        })
        .unwrap();
        assert_eq!(r.verb, HttpVerb::Delete);
    }

    #[test]
    fn scan_image() {
        let r = prepare(&HarborCmd::Scan {
            image: "library/nginx:1.27".into(),
        })
        .unwrap();
        assert!(r.path.ends_with("/artifacts/1.27/scan"));
    }

    #[test]
    fn gc_dry_run() {
        let r = prepare(&HarborCmd::Gc { dry_run: true }).unwrap();
        assert_eq!(r.body.unwrap()["dry_run"], true);
    }

    #[test]
    fn gc_real() {
        let r = prepare(&HarborCmd::Gc { dry_run: false }).unwrap();
        assert_eq!(r.body.unwrap()["dry_run"], false);
    }

    #[test]
    fn paths_use_compat_harbor_prefix() {
        let r = prepare(&HarborCmd::Project {
            cmd: ProjectCmd::List,
        })
        .unwrap();
        assert!(r.path.starts_with("/api/compat/harbor/v2/"));
    }
}
