// SPDX-License-Identifier: AGPL-3.0-or-later
//! `cavectl helm …` — Helm v3 compat shim.
//!
//! Targets `/api/compat/helm/v3/...`. Cave's registry+gitops modules
//! own the actual chart resolution; this shim translates the helm
//! verb shape into routed requests.

use anyhow::{bail, Result};
use clap::Subcommand;
use serde_json::{json, Value};

use crate::native::{HttpVerb, PreparedRequest};

#[derive(Subcommand, Debug, Clone)]
pub enum HelmCmd {
    Install {
        release: String,
        chart: String,
        #[arg(short = 'n', long)]
        namespace: Option<String>,
        #[arg(long)]
        version: Option<String>,
        #[arg(long = "set", num_args = 1..)]
        set: Vec<String>,
        #[arg(short = 'f', long = "values")]
        values: Vec<String>,
        #[arg(long)]
        wait: bool,
        #[arg(long = "create-namespace")]
        create_namespace: bool,
    },
    Upgrade {
        release: String,
        chart: String,
        #[arg(short = 'n', long)]
        namespace: Option<String>,
        #[arg(long)]
        version: Option<String>,
        #[arg(long = "set", num_args = 1..)]
        set: Vec<String>,
        #[arg(short = 'f', long = "values")]
        values: Vec<String>,
        #[arg(long)]
        install: bool,
        #[arg(long)]
        atomic: bool,
    },
    Uninstall {
        release: String,
        #[arg(short = 'n', long)]
        namespace: Option<String>,
        #[arg(long = "keep-history")]
        keep_history: bool,
    },
    List {
        #[arg(short = 'n', long)]
        namespace: Option<String>,
        #[arg(short = 'A', long = "all-namespaces")]
        all_namespaces: bool,
        #[arg(long)]
        deployed: bool,
        #[arg(long)]
        failed: bool,
    },
    Get {
        what: String,
        release: String,
        #[arg(short = 'n', long)]
        namespace: Option<String>,
    },
    Rollback {
        release: String,
        revision: u32,
        #[arg(short = 'n', long)]
        namespace: Option<String>,
    },
    Repo {
        #[command(subcommand)]
        cmd: RepoCmd,
    },
    Search {
        keyword: String,
        #[arg(long)]
        regex: bool,
    },
}

#[derive(Subcommand, Debug, Clone)]
pub enum RepoCmd {
    Add { name: String, url: String },
    Remove { name: String },
    List,
    Update,
}

const GET_KINDS: &[&str] = &["values", "manifest", "notes", "hooks", "all"];

pub fn prepare(cmd: &HelmCmd) -> Result<PreparedRequest> {
    match cmd {
        HelmCmd::Install {
            release,
            chart,
            namespace,
            version,
            set,
            values,
            wait,
            create_namespace,
        } => {
            validate_release(release)?;
            let mut body: Value = json!({
                "release": release,
                "chart": chart,
            });
            if let Some(v) = version {
                body["version"] = json!(v);
            }
            if !set.is_empty() {
                body["set"] = json!(set);
            }
            if !values.is_empty() {
                body["values_files"] = json!(values);
            }
            if *wait {
                body["wait"] = json!(true);
            }
            if *create_namespace {
                body["create_namespace"] = json!(true);
            }
            let path = ns_release(namespace.as_deref(), None);
            Ok(PreparedRequest::new(HttpVerb::Post, format!("{}/install", path)).with_body(body))
        }
        HelmCmd::Upgrade {
            release,
            chart,
            namespace,
            version,
            set,
            values,
            install,
            atomic,
        } => {
            validate_release(release)?;
            let mut body: Value = json!({
                "release": release,
                "chart": chart,
            });
            if let Some(v) = version {
                body["version"] = json!(v);
            }
            if !set.is_empty() {
                body["set"] = json!(set);
            }
            if !values.is_empty() {
                body["values_files"] = json!(values);
            }
            if *install {
                body["install_if_missing"] = json!(true);
            }
            if *atomic {
                body["atomic"] = json!(true);
            }
            let path = ns_release(namespace.as_deref(), Some(release));
            Ok(PreparedRequest::new(HttpVerb::Put, path).with_body(body))
        }
        HelmCmd::Uninstall {
            release,
            namespace,
            keep_history,
        } => {
            validate_release(release)?;
            let mut path = ns_release(namespace.as_deref(), Some(release));
            if *keep_history {
                path.push_str("?keep_history=true");
            }
            Ok(PreparedRequest::new(HttpVerb::Delete, path))
        }
        HelmCmd::List {
            namespace,
            all_namespaces,
            deployed,
            failed,
        } => {
            let mut path = if *all_namespaces {
                "/api/compat/helm/v3/all/releases".to_string()
            } else {
                ns_release(namespace.as_deref(), None)
            };
            let mut params: Vec<String> = Vec::new();
            if *deployed {
                params.push("filter=deployed".to_string());
            }
            if *failed {
                params.push("filter=failed".to_string());
            }
            if !params.is_empty() {
                path.push('?');
                path.push_str(&params.join("&"));
            }
            Ok(PreparedRequest::new(HttpVerb::Get, path))
        }
        HelmCmd::Get {
            what,
            release,
            namespace,
        } => {
            if !GET_KINDS.contains(&what.as_str()) {
                bail!("unknown `get` kind `{}`; want one of {:?}", what, GET_KINDS);
            }
            validate_release(release)?;
            let path = format!(
                "{}/{}",
                ns_release(namespace.as_deref(), Some(release)),
                what
            );
            Ok(PreparedRequest::new(HttpVerb::Get, path))
        }
        HelmCmd::Rollback {
            release,
            revision,
            namespace,
        } => {
            validate_release(release)?;
            let path = format!(
                "{}/rollback/{}",
                ns_release(namespace.as_deref(), Some(release)),
                revision
            );
            Ok(PreparedRequest::new(HttpVerb::Post, path).with_body(json!({})))
        }
        HelmCmd::Repo { cmd } => prepare_repo(cmd),
        HelmCmd::Search { keyword, regex } => {
            if keyword.is_empty() {
                bail!("search keyword required");
            }
            let mut path = format!("/api/compat/helm/v3/search?q={}", keyword);
            if *regex {
                path.push_str("&regex=true");
            }
            Ok(PreparedRequest::new(HttpVerb::Get, path))
        }
    }
}

fn ns_release(namespace: Option<&str>, release: Option<&str>) -> String {
    let ns = namespace.unwrap_or("default");
    match release {
        Some(r) => format!("/api/compat/helm/v3/namespaces/{}/releases/{}", ns, r),
        None => format!("/api/compat/helm/v3/namespaces/{}/releases", ns),
    }
}

fn prepare_repo(cmd: &RepoCmd) -> Result<PreparedRequest> {
    match cmd {
        RepoCmd::Add { name, url } => {
            if name.is_empty() || url.is_empty() {
                bail!("repo name and url required");
            }
            Ok(
                PreparedRequest::new(HttpVerb::Post, "/api/compat/helm/v3/repos")
                    .with_body(json!({"name": name, "url": url})),
            )
        }
        RepoCmd::Remove { name } => Ok(PreparedRequest::new(
            HttpVerb::Delete,
            format!("/api/compat/helm/v3/repos/{}", name),
        )),
        RepoCmd::List => Ok(PreparedRequest::new(
            HttpVerb::Get,
            "/api/compat/helm/v3/repos",
        )),
        RepoCmd::Update => Ok(PreparedRequest::new(
            HttpVerb::Post,
            "/api/compat/helm/v3/repos/update",
        )
        .with_body(json!({}))),
    }
}

fn validate_release(r: &str) -> Result<()> {
    if r.is_empty() {
        bail!("release name required");
    }
    if r.len() > 53 {
        bail!("release name too long (max 53)");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn install(release: &str, chart: &str) -> HelmCmd {
        HelmCmd::Install {
            release: release.into(),
            chart: chart.into(),
            namespace: None,
            version: None,
            set: vec![],
            values: vec![],
            wait: false,
            create_namespace: false,
        }
    }

    #[test]
    fn install_basic() {
        let r = prepare(&install("redis", "bitnami/redis")).unwrap();
        assert_eq!(r.verb, HttpVerb::Post);
        assert!(r.path.ends_with("/releases/install"));
        let body = r.body.unwrap();
        assert_eq!(body["release"], "redis");
        assert_eq!(body["chart"], "bitnami/redis");
    }

    #[test]
    fn install_with_namespace() {
        let mut c = install("redis", "bitnami/redis");
        if let HelmCmd::Install { namespace, .. } = &mut c {
            *namespace = Some("data".into());
        }
        let r = prepare(&c).unwrap();
        assert!(r.path.contains("/namespaces/data/"));
    }

    #[test]
    fn install_with_set_values() {
        let mut c = install("r", "c");
        if let HelmCmd::Install { set, .. } = &mut c {
            *set = vec!["a=1".into(), "b=2".into()];
        }
        let body = prepare(&c).unwrap().body.unwrap();
        assert_eq!(body["set"][0], "a=1");
        assert_eq!(body["set"][1], "b=2");
    }

    #[test]
    fn install_wait_and_create_ns() {
        let c = HelmCmd::Install {
            release: "r".into(),
            chart: "c".into(),
            namespace: None,
            version: None,
            set: vec![],
            values: vec![],
            wait: true,
            create_namespace: true,
        };
        let body = prepare(&c).unwrap().body.unwrap();
        assert_eq!(body["wait"], true);
        assert_eq!(body["create_namespace"], true);
    }

    #[test]
    fn install_rejects_empty_release() {
        let c = install("", "c");
        assert!(prepare(&c).is_err());
    }

    #[test]
    fn install_rejects_too_long_release() {
        let big = "a".repeat(54);
        let c = install(&big, "c");
        assert!(prepare(&c).is_err());
    }

    #[test]
    fn upgrade_uses_put() {
        let c = HelmCmd::Upgrade {
            release: "r".into(),
            chart: "c".into(),
            namespace: None,
            version: None,
            set: vec![],
            values: vec![],
            install: false,
            atomic: false,
        };
        let r = prepare(&c).unwrap();
        assert_eq!(r.verb, HttpVerb::Put);
        assert!(r.path.contains("/releases/r"));
    }

    #[test]
    fn upgrade_install_flag() {
        let c = HelmCmd::Upgrade {
            release: "r".into(),
            chart: "c".into(),
            namespace: None,
            version: None,
            set: vec![],
            values: vec![],
            install: true,
            atomic: true,
        };
        let body = prepare(&c).unwrap().body.unwrap();
        assert_eq!(body["install_if_missing"], true);
        assert_eq!(body["atomic"], true);
    }

    #[test]
    fn uninstall() {
        let c = HelmCmd::Uninstall {
            release: "r".into(),
            namespace: None,
            keep_history: false,
        };
        let r = prepare(&c).unwrap();
        assert_eq!(r.verb, HttpVerb::Delete);
        assert!(r.path.contains("/releases/r"));
    }

    #[test]
    fn uninstall_keep_history() {
        let c = HelmCmd::Uninstall {
            release: "r".into(),
            namespace: None,
            keep_history: true,
        };
        let r = prepare(&c).unwrap();
        assert!(r.path.contains("keep_history=true"));
    }

    #[test]
    fn list_default() {
        let c = HelmCmd::List {
            namespace: None,
            all_namespaces: false,
            deployed: false,
            failed: false,
        };
        let r = prepare(&c).unwrap();
        assert_eq!(r.path, "/api/compat/helm/v3/namespaces/default/releases");
    }

    #[test]
    fn list_all_namespaces() {
        let c = HelmCmd::List {
            namespace: None,
            all_namespaces: true,
            deployed: false,
            failed: false,
        };
        assert_eq!(prepare(&c).unwrap().path, "/api/compat/helm/v3/all/releases");
    }

    #[test]
    fn list_deployed_filter() {
        let c = HelmCmd::List {
            namespace: None,
            all_namespaces: false,
            deployed: true,
            failed: false,
        };
        assert!(prepare(&c).unwrap().path.contains("filter=deployed"));
    }

    #[test]
    fn get_values() {
        let c = HelmCmd::Get {
            what: "values".into(),
            release: "r".into(),
            namespace: None,
        };
        let r = prepare(&c).unwrap();
        assert!(r.path.ends_with("/releases/r/values"));
    }

    #[test]
    fn get_kinds_round_trip() {
        for k in GET_KINDS {
            let c = HelmCmd::Get {
                what: (*k).into(),
                release: "r".into(),
                namespace: None,
            };
            assert!(prepare(&c).is_ok(), "kind {} should be accepted", k);
        }
    }

    #[test]
    fn get_rejects_unknown_kind() {
        let c = HelmCmd::Get {
            what: "foo".into(),
            release: "r".into(),
            namespace: None,
        };
        assert!(prepare(&c).is_err());
    }

    #[test]
    fn rollback() {
        let c = HelmCmd::Rollback {
            release: "r".into(),
            revision: 3,
            namespace: None,
        };
        let r = prepare(&c).unwrap();
        assert_eq!(r.verb, HttpVerb::Post);
        assert!(r.path.contains("/rollback/3"));
    }

    #[test]
    fn repo_add() {
        let c = HelmCmd::Repo {
            cmd: RepoCmd::Add {
                name: "bitnami".into(),
                url: "https://charts.bitnami.com/bitnami".into(),
            },
        };
        let r = prepare(&c).unwrap();
        assert_eq!(r.verb, HttpVerb::Post);
        assert_eq!(r.path, "/api/compat/helm/v3/repos");
        let body = r.body.unwrap();
        assert_eq!(body["name"], "bitnami");
    }

    #[test]
    fn repo_add_rejects_empty_name() {
        let c = HelmCmd::Repo {
            cmd: RepoCmd::Add {
                name: "".into(),
                url: "x".into(),
            },
        };
        assert!(prepare(&c).is_err());
    }

    #[test]
    fn repo_remove() {
        let c = HelmCmd::Repo {
            cmd: RepoCmd::Remove {
                name: "bitnami".into(),
            },
        };
        let r = prepare(&c).unwrap();
        assert_eq!(r.verb, HttpVerb::Delete);
    }

    #[test]
    fn repo_list() {
        let c = HelmCmd::Repo { cmd: RepoCmd::List };
        let r = prepare(&c).unwrap();
        assert_eq!(r.verb, HttpVerb::Get);
    }

    #[test]
    fn repo_update() {
        let c = HelmCmd::Repo {
            cmd: RepoCmd::Update,
        };
        let r = prepare(&c).unwrap();
        assert_eq!(r.verb, HttpVerb::Post);
        assert!(r.path.ends_with("/repos/update"));
    }

    #[test]
    fn search() {
        let c = HelmCmd::Search {
            keyword: "redis".into(),
            regex: false,
        };
        let r = prepare(&c).unwrap();
        assert!(r.path.contains("q=redis"));
    }

    #[test]
    fn search_regex() {
        let c = HelmCmd::Search {
            keyword: "^redis$".into(),
            regex: true,
        };
        assert!(prepare(&c).unwrap().path.contains("regex=true"));
    }

    #[test]
    fn search_rejects_empty() {
        let c = HelmCmd::Search {
            keyword: "".into(),
            regex: false,
        };
        assert!(prepare(&c).is_err());
    }

    #[test]
    fn install_with_version() {
        let mut c = install("r", "c");
        if let HelmCmd::Install { version, .. } = &mut c {
            *version = Some("1.2.3".into());
        }
        let body = prepare(&c).unwrap().body.unwrap();
        assert_eq!(body["version"], "1.2.3");
    }

    #[test]
    fn install_with_values_files() {
        let mut c = install("r", "c");
        if let HelmCmd::Install { values, .. } = &mut c {
            *values = vec!["v1.yaml".into(), "v2.yaml".into()];
        }
        let body = prepare(&c).unwrap().body.unwrap();
        assert_eq!(body["values_files"][0], "v1.yaml");
        assert_eq!(body["values_files"][1], "v2.yaml");
    }

    #[test]
    fn paths_use_compat_helm_prefix() {
        let r = prepare(&install("r", "c")).unwrap();
        assert!(r.path.starts_with("/api/compat/helm/v3/"));
    }
}
