// SPDX-License-Identifier: AGPL-3.0-or-later
//! `cavectl argocd …` — Argo CD compat shim.

use anyhow::{bail, Result};
use clap::Subcommand;
use serde_json::{json, Value};

use crate::native::{HttpVerb, PreparedRequest};

#[derive(Subcommand, Debug, Clone)]
pub enum ArgoCdCmd {
    App {
        #[command(subcommand)]
        cmd: AppCmd,
    },
    Repo {
        #[command(subcommand)]
        cmd: RepoCmd,
    },
    Cluster {
        #[command(subcommand)]
        cmd: ClusterCmd,
    },
    Project {
        #[command(subcommand)]
        cmd: ProjectCmd,
    },
}

#[derive(Subcommand, Debug, Clone)]
pub enum AppCmd {
    Create {
        name: String,
        #[arg(long = "repo")]
        repo: String,
        #[arg(long = "path")]
        path: String,
        #[arg(long = "dest-server")]
        dest_server: String,
        #[arg(long = "dest-namespace")]
        dest_namespace: String,
        #[arg(long = "revision", default_value = "HEAD")]
        revision: String,
        #[arg(long = "auto-sync")]
        auto_sync: bool,
        #[arg(long = "self-heal")]
        self_heal: bool,
    },
    List {
        #[arg(long)]
        project: Option<String>,
        #[arg(long)]
        cluster: Option<String>,
    },
    Get {
        name: String,
        #[arg(long = "show-operation")]
        show_operation: bool,
    },
    Sync {
        name: String,
        #[arg(long)]
        prune: bool,
        #[arg(long)]
        force: bool,
        #[arg(long = "dry-run")]
        dry_run: bool,
        #[arg(long = "revision")]
        revision: Option<String>,
    },
    Diff {
        name: String,
    },
    Rollback {
        name: String,
        history_id: u32,
    },
    Delete {
        name: String,
        #[arg(long)]
        cascade: bool,
    },
    History {
        name: String,
    },
    Set {
        name: String,
        #[arg(long, num_args = 1..)]
        parameter: Vec<String>,
    },
}

#[derive(Subcommand, Debug, Clone)]
pub enum RepoCmd {
    Add {
        url: String,
        #[arg(long)]
        username: Option<String>,
    },
    Remove {
        url: String,
    },
    List,
}

#[derive(Subcommand, Debug, Clone)]
pub enum ClusterCmd {
    Add {
        context: String,
        #[arg(long)]
        name: Option<String>,
    },
    Remove {
        server: String,
    },
    List,
}

#[derive(Subcommand, Debug, Clone)]
pub enum ProjectCmd {
    Create {
        name: String,
    },
    Delete {
        name: String,
    },
    List,
}

pub fn prepare(cmd: &ArgoCdCmd) -> Result<PreparedRequest> {
    match cmd {
        ArgoCdCmd::App { cmd } => prepare_app(cmd),
        ArgoCdCmd::Repo { cmd } => prepare_repo(cmd),
        ArgoCdCmd::Cluster { cmd } => prepare_cluster(cmd),
        ArgoCdCmd::Project { cmd } => prepare_project(cmd),
    }
}

fn prepare_app(cmd: &AppCmd) -> Result<PreparedRequest> {
    match cmd {
        AppCmd::Create {
            name,
            repo,
            path,
            dest_server,
            dest_namespace,
            revision,
            auto_sync,
            self_heal,
        } => {
            validate_name(name)?;
            let mut body: Value = json!({
                "name": name,
                "spec": {
                    "source": {
                        "repoURL": repo,
                        "path": path,
                        "targetRevision": revision,
                    },
                    "destination": {
                        "server": dest_server,
                        "namespace": dest_namespace,
                    },
                },
            });
            if *auto_sync || *self_heal {
                body["spec"]["syncPolicy"] = json!({
                    "automated": {
                        "prune": *auto_sync,
                        "selfHeal": *self_heal,
                    }
                });
            }
            Ok(
                PreparedRequest::new(HttpVerb::Post, "/api/compat/argocd/v1/applications")
                    .with_body(body),
            )
        }
        AppCmd::List { project, cluster } => {
            let mut path = "/api/compat/argocd/v1/applications".to_string();
            let mut params: Vec<String> = Vec::new();
            if let Some(p) = project {
                params.push(format!("project={}", p));
            }
            if let Some(c) = cluster {
                params.push(format!("cluster={}", c));
            }
            if !params.is_empty() {
                path.push('?');
                path.push_str(&params.join("&"));
            }
            Ok(PreparedRequest::new(HttpVerb::Get, path))
        }
        AppCmd::Get {
            name,
            show_operation,
        } => {
            validate_name(name)?;
            let mut path = format!("/api/compat/argocd/v1/applications/{}", name);
            if *show_operation {
                path.push_str("?showOperation=true");
            }
            Ok(PreparedRequest::new(HttpVerb::Get, path))
        }
        AppCmd::Sync {
            name,
            prune,
            force,
            dry_run,
            revision,
        } => {
            validate_name(name)?;
            let mut body: Value = json!({});
            if *prune {
                body["prune"] = json!(true);
            }
            if *force {
                body["force"] = json!(true);
            }
            if *dry_run {
                body["dryRun"] = json!(true);
            }
            if let Some(r) = revision {
                body["revision"] = json!(r);
            }
            Ok(PreparedRequest::new(
                HttpVerb::Post,
                format!("/api/compat/argocd/v1/applications/{}/sync", name),
            )
            .with_body(body))
        }
        AppCmd::Diff { name } => {
            validate_name(name)?;
            Ok(PreparedRequest::new(
                HttpVerb::Get,
                format!("/api/compat/argocd/v1/applications/{}/diff", name),
            ))
        }
        AppCmd::Rollback { name, history_id } => {
            validate_name(name)?;
            Ok(PreparedRequest::new(
                HttpVerb::Post,
                format!(
                    "/api/compat/argocd/v1/applications/{}/rollback/{}",
                    name, history_id
                ),
            )
            .with_body(json!({})))
        }
        AppCmd::Delete { name, cascade } => {
            validate_name(name)?;
            let mut path = format!("/api/compat/argocd/v1/applications/{}", name);
            if *cascade {
                path.push_str("?cascade=true");
            }
            Ok(PreparedRequest::new(HttpVerb::Delete, path))
        }
        AppCmd::History { name } => {
            validate_name(name)?;
            Ok(PreparedRequest::new(
                HttpVerb::Get,
                format!("/api/compat/argocd/v1/applications/{}/history", name),
            ))
        }
        AppCmd::Set { name, parameter } => {
            validate_name(name)?;
            if parameter.is_empty() {
                bail!("at least one --parameter required");
            }
            let body = json!({"parameters": parameter});
            Ok(PreparedRequest::new(
                HttpVerb::Patch,
                format!("/api/compat/argocd/v1/applications/{}", name),
            )
            .with_body(body))
        }
    }
}

fn prepare_repo(cmd: &RepoCmd) -> Result<PreparedRequest> {
    match cmd {
        RepoCmd::Add { url, username } => {
            if url.is_empty() {
                bail!("repo url required");
            }
            let mut body: Value = json!({"url": url});
            if let Some(u) = username {
                body["username"] = json!(u);
            }
            Ok(
                PreparedRequest::new(HttpVerb::Post, "/api/compat/argocd/v1/repos")
                    .with_body(body),
            )
        }
        RepoCmd::Remove { url } => Ok(PreparedRequest::new(
            HttpVerb::Delete,
            format!(
                "/api/compat/argocd/v1/repos/{}",
                url_segment_safe(url)
            ),
        )),
        RepoCmd::List => Ok(PreparedRequest::new(
            HttpVerb::Get,
            "/api/compat/argocd/v1/repos",
        )),
    }
}

fn prepare_cluster(cmd: &ClusterCmd) -> Result<PreparedRequest> {
    match cmd {
        ClusterCmd::Add { context, name } => {
            if context.is_empty() {
                bail!("kube context required");
            }
            let mut body: Value = json!({"context": context});
            if let Some(n) = name {
                body["name"] = json!(n);
            }
            Ok(
                PreparedRequest::new(HttpVerb::Post, "/api/compat/argocd/v1/clusters")
                    .with_body(body),
            )
        }
        ClusterCmd::Remove { server } => Ok(PreparedRequest::new(
            HttpVerb::Delete,
            format!(
                "/api/compat/argocd/v1/clusters/{}",
                url_segment_safe(server)
            ),
        )),
        ClusterCmd::List => Ok(PreparedRequest::new(
            HttpVerb::Get,
            "/api/compat/argocd/v1/clusters",
        )),
    }
}

fn prepare_project(cmd: &ProjectCmd) -> Result<PreparedRequest> {
    match cmd {
        ProjectCmd::Create { name } => {
            validate_name(name)?;
            Ok(
                PreparedRequest::new(HttpVerb::Post, "/api/compat/argocd/v1/projects")
                    .with_body(json!({"name": name})),
            )
        }
        ProjectCmd::Delete { name } => {
            validate_name(name)?;
            Ok(PreparedRequest::new(
                HttpVerb::Delete,
                format!("/api/compat/argocd/v1/projects/{}", name),
            ))
        }
        ProjectCmd::List => Ok(PreparedRequest::new(
            HttpVerb::Get,
            "/api/compat/argocd/v1/projects",
        )),
    }
}

fn validate_name(n: &str) -> Result<()> {
    if n.is_empty() {
        bail!("name required");
    }
    if n.len() > 63 {
        bail!("name too long (max 63)");
    }
    Ok(())
}

fn url_segment_safe(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char);
            }
            _ => {
                use std::fmt::Write;
                let _ = write!(out, "%{:02X}", b);
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn create() -> AppCmd {
        AppCmd::Create {
            name: "guestbook".into(),
            repo: "https://github.com/argoproj/argocd-example-apps".into(),
            path: "guestbook".into(),
            dest_server: "https://kubernetes.default.svc".into(),
            dest_namespace: "default".into(),
            revision: "HEAD".into(),
            auto_sync: false,
            self_heal: false,
        }
    }

    #[test]
    fn app_create_basic() {
        let r = prepare(&ArgoCdCmd::App { cmd: create() }).unwrap();
        assert_eq!(r.verb, HttpVerb::Post);
        assert_eq!(r.path, "/api/compat/argocd/v1/applications");
        let body = r.body.unwrap();
        assert_eq!(body["name"], "guestbook");
        assert_eq!(body["spec"]["source"]["path"], "guestbook");
    }

    #[test]
    fn app_create_with_auto_sync_self_heal() {
        let mut c = create();
        if let AppCmd::Create {
            auto_sync,
            self_heal,
            ..
        } = &mut c
        {
            *auto_sync = true;
            *self_heal = true;
        }
        let body = prepare(&ArgoCdCmd::App { cmd: c }).unwrap().body.unwrap();
        assert_eq!(body["spec"]["syncPolicy"]["automated"]["prune"], true);
        assert_eq!(body["spec"]["syncPolicy"]["automated"]["selfHeal"], true);
    }

    #[test]
    fn app_create_default_revision_head() {
        let body = prepare(&ArgoCdCmd::App { cmd: create() })
            .unwrap()
            .body
            .unwrap();
        assert_eq!(body["spec"]["source"]["targetRevision"], "HEAD");
    }

    #[test]
    fn app_create_rejects_empty_name() {
        let mut c = create();
        if let AppCmd::Create { name, .. } = &mut c {
            *name = "".into();
        }
        assert!(prepare(&ArgoCdCmd::App { cmd: c }).is_err());
    }

    #[test]
    fn app_list_no_filter() {
        let c = ArgoCdCmd::App {
            cmd: AppCmd::List {
                project: None,
                cluster: None,
            },
        };
        assert_eq!(
            prepare(&c).unwrap().path,
            "/api/compat/argocd/v1/applications"
        );
    }

    #[test]
    fn app_list_filtered() {
        let c = ArgoCdCmd::App {
            cmd: AppCmd::List {
                project: Some("default".into()),
                cluster: Some("dev".into()),
            },
        };
        let p = prepare(&c).unwrap().path;
        assert!(p.contains("project=default"));
        assert!(p.contains("cluster=dev"));
    }

    #[test]
    fn app_get() {
        let c = ArgoCdCmd::App {
            cmd: AppCmd::Get {
                name: "guestbook".into(),
                show_operation: false,
            },
        };
        let r = prepare(&c).unwrap();
        assert_eq!(r.verb, HttpVerb::Get);
        assert_eq!(r.path, "/api/compat/argocd/v1/applications/guestbook");
    }

    #[test]
    fn app_get_show_operation() {
        let c = ArgoCdCmd::App {
            cmd: AppCmd::Get {
                name: "g".into(),
                show_operation: true,
            },
        };
        assert!(prepare(&c).unwrap().path.contains("showOperation=true"));
    }

    #[test]
    fn app_sync() {
        let c = ArgoCdCmd::App {
            cmd: AppCmd::Sync {
                name: "g".into(),
                prune: false,
                force: false,
                dry_run: false,
                revision: None,
            },
        };
        let r = prepare(&c).unwrap();
        assert_eq!(r.verb, HttpVerb::Post);
        assert!(r.path.ends_with("/applications/g/sync"));
    }

    #[test]
    fn app_sync_with_flags() {
        let c = ArgoCdCmd::App {
            cmd: AppCmd::Sync {
                name: "g".into(),
                prune: true,
                force: true,
                dry_run: true,
                revision: Some("abc".into()),
            },
        };
        let body = prepare(&c).unwrap().body.unwrap();
        assert_eq!(body["prune"], true);
        assert_eq!(body["force"], true);
        assert_eq!(body["dryRun"], true);
        assert_eq!(body["revision"], "abc");
    }

    #[test]
    fn app_diff() {
        let c = ArgoCdCmd::App {
            cmd: AppCmd::Diff {
                name: "g".into(),
            },
        };
        let r = prepare(&c).unwrap();
        assert_eq!(r.verb, HttpVerb::Get);
        assert!(r.path.ends_with("/g/diff"));
    }

    #[test]
    fn app_rollback() {
        let c = ArgoCdCmd::App {
            cmd: AppCmd::Rollback {
                name: "g".into(),
                history_id: 5,
            },
        };
        let r = prepare(&c).unwrap();
        assert_eq!(r.verb, HttpVerb::Post);
        assert!(r.path.ends_with("/rollback/5"));
    }

    #[test]
    fn app_delete_cascade() {
        let c = ArgoCdCmd::App {
            cmd: AppCmd::Delete {
                name: "g".into(),
                cascade: true,
            },
        };
        let r = prepare(&c).unwrap();
        assert_eq!(r.verb, HttpVerb::Delete);
        assert!(r.path.contains("cascade=true"));
    }

    #[test]
    fn app_history() {
        let c = ArgoCdCmd::App {
            cmd: AppCmd::History {
                name: "g".into(),
            },
        };
        let r = prepare(&c).unwrap();
        assert!(r.path.ends_with("/history"));
    }

    #[test]
    fn app_set_parameter() {
        let c = ArgoCdCmd::App {
            cmd: AppCmd::Set {
                name: "g".into(),
                parameter: vec!["image.tag=v2".into()],
            },
        };
        let r = prepare(&c).unwrap();
        assert_eq!(r.verb, HttpVerb::Patch);
        let body = r.body.unwrap();
        assert_eq!(body["parameters"][0], "image.tag=v2");
    }

    #[test]
    fn app_set_rejects_no_parameter() {
        let c = ArgoCdCmd::App {
            cmd: AppCmd::Set {
                name: "g".into(),
                parameter: vec![],
            },
        };
        assert!(prepare(&c).is_err());
    }

    #[test]
    fn repo_add() {
        let c = ArgoCdCmd::Repo {
            cmd: RepoCmd::Add {
                url: "https://x".into(),
                username: Some("bot".into()),
            },
        };
        let r = prepare(&c).unwrap();
        assert_eq!(r.path, "/api/compat/argocd/v1/repos");
        assert_eq!(r.body.unwrap()["username"], "bot");
    }

    #[test]
    fn repo_add_rejects_empty_url() {
        let c = ArgoCdCmd::Repo {
            cmd: RepoCmd::Add {
                url: "".into(),
                username: None,
            },
        };
        assert!(prepare(&c).is_err());
    }

    #[test]
    fn repo_remove_url_safe() {
        let c = ArgoCdCmd::Repo {
            cmd: RepoCmd::Remove {
                url: "https://x".into(),
            },
        };
        let r = prepare(&c).unwrap();
        assert!(r.path.contains("https%3A%2F%2Fx"));
    }

    #[test]
    fn repo_list() {
        let c = ArgoCdCmd::Repo { cmd: RepoCmd::List };
        let r = prepare(&c).unwrap();
        assert_eq!(r.verb, HttpVerb::Get);
    }

    #[test]
    fn cluster_add() {
        let c = ArgoCdCmd::Cluster {
            cmd: ClusterCmd::Add {
                context: "dev-cluster".into(),
                name: None,
            },
        };
        let r = prepare(&c).unwrap();
        assert_eq!(r.body.unwrap()["context"], "dev-cluster");
    }

    #[test]
    fn cluster_remove() {
        let c = ArgoCdCmd::Cluster {
            cmd: ClusterCmd::Remove {
                server: "https://x".into(),
            },
        };
        let r = prepare(&c).unwrap();
        assert_eq!(r.verb, HttpVerb::Delete);
    }

    #[test]
    fn cluster_list() {
        let c = ArgoCdCmd::Cluster {
            cmd: ClusterCmd::List,
        };
        let r = prepare(&c).unwrap();
        assert_eq!(r.path, "/api/compat/argocd/v1/clusters");
    }

    #[test]
    fn project_create() {
        let c = ArgoCdCmd::Project {
            cmd: ProjectCmd::Create {
                name: "p".into(),
            },
        };
        let r = prepare(&c).unwrap();
        assert_eq!(r.verb, HttpVerb::Post);
    }

    #[test]
    fn project_delete() {
        let c = ArgoCdCmd::Project {
            cmd: ProjectCmd::Delete {
                name: "p".into(),
            },
        };
        let r = prepare(&c).unwrap();
        assert_eq!(r.verb, HttpVerb::Delete);
    }

    #[test]
    fn project_list() {
        let c = ArgoCdCmd::Project {
            cmd: ProjectCmd::List,
        };
        let r = prepare(&c).unwrap();
        assert_eq!(r.verb, HttpVerb::Get);
    }

    #[test]
    fn paths_use_compat_argocd_prefix() {
        let r = prepare(&ArgoCdCmd::App { cmd: create() }).unwrap();
        assert!(r.path.starts_with("/api/compat/argocd/v1/"));
    }
}
