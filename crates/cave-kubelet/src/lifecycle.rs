//! Container lifecycle hooks — PostStart and PreStop.
//!
//! Mirrors `pkg/kubelet/lifecycle` and `pkg/api/v1/pod/util.go`: handler
//! kinds (Exec, HTTPGet, TCPSocket, Sleep), validation rules, and
//! handler dispatch decisions including the kubelet's PreStop hook timing
//! against the pod's terminationGracePeriodSeconds.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Handler {
    Exec(ExecHandler),
    HttpGet(HttpGetHandler),
    TcpSocket(TcpSocketHandler),
    /// KEP-3960 Sleep action (alpha 1.29, beta 1.30).
    Sleep(SleepHandler),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExecHandler {
    pub command: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HttpGetHandler {
    pub scheme: HttpScheme,
    pub host: Option<String>,
    pub port: u16,
    pub path: String,
    pub http_headers: Vec<(String, String)>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum HttpScheme {
    Http,
    Https,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TcpSocketHandler {
    pub host: Option<String>,
    pub port: u16,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct SleepHandler {
    pub seconds: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum HookKind {
    PostStart,
    PreStop,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LifecycleHook {
    pub kind: HookKind,
    pub handler: Handler,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum LifecycleError {
    #[error("invalid: {0}")]
    Invalid(String),
}

pub type LcResult<T> = Result<T, LifecycleError>;

pub fn validate_handler(h: &Handler) -> LcResult<()> {
    match h {
        Handler::Exec(e) => {
            if e.command.is_empty() {
                return Err(LifecycleError::Invalid("exec command empty".into()));
            }
            Ok(())
        }
        Handler::HttpGet(g) => {
            if g.port == 0 {
                return Err(LifecycleError::Invalid("httpGet port 0".into()));
            }
            if g.path.is_empty() {
                return Err(LifecycleError::Invalid("httpGet path empty".into()));
            }
            // Header names must be ASCII tokens.
            for (k, _) in &g.http_headers {
                if k.is_empty()
                    || !k.chars().all(|c| {
                        c.is_ascii_alphanumeric() || matches!(c, '-' | '_')
                    })
                {
                    return Err(LifecycleError::Invalid(format!(
                        "invalid HTTP header name: {}",
                        k
                    )));
                }
            }
            Ok(())
        }
        Handler::TcpSocket(t) => {
            if t.port == 0 {
                return Err(LifecycleError::Invalid("tcpSocket port 0".into()));
            }
            Ok(())
        }
        Handler::Sleep(s) => {
            if s.seconds < 0 {
                return Err(LifecycleError::Invalid("sleep seconds negative".into()));
            }
            Ok(())
        }
    }
}

/// Kubelet's classification: whether a hook handler is *non-blocking*
/// (HttpGet/TcpSocket return after first byte) vs blocks for the duration
/// (Exec, Sleep). Used to compute PreStop's effective deadline.
pub fn is_blocking(h: &Handler) -> bool {
    matches!(h, Handler::Exec(_) | Handler::Sleep(_))
}

/// Compute the effective wait time for a PreStop hook before SIGTERM is
/// dispatched. Per upstream `kubelet/container/runtime.go` `executePreStopHook`
/// the kubelet waits for the hook to complete, but bounded by the pod's
/// `terminationGracePeriodSeconds`. Returns the bound that the kubelet uses.
pub fn pre_stop_deadline_seconds(
    handler: &Handler,
    termination_grace_period_seconds: i64,
) -> i64 {
    let h_max = match handler {
        Handler::Sleep(s) => s.seconds,
        // Exec / HTTP / TCP don't pre-declare a duration; fall back to grace.
        _ => termination_grace_period_seconds,
    };
    h_max.min(termination_grace_period_seconds.max(0))
}

/// SIGTERM is sent immediately after a PreStop hook returns OR when the
/// pre-stop deadline elapses. SIGKILL follows after the remaining grace.
/// Returns (sigterm_at, sigkill_at) seconds from t=0 (start of termination).
pub fn termination_schedule(
    pre_stop: Option<&Handler>,
    termination_grace_period_seconds: i64,
) -> (i64, i64) {
    if termination_grace_period_seconds <= 0 {
        // Force-kill immediately.
        return (0, 0);
    }
    let prestop_deadline = match pre_stop {
        None => 0,
        Some(h) => pre_stop_deadline_seconds(h, termination_grace_period_seconds),
    };
    let sigterm_at = prestop_deadline.min(termination_grace_period_seconds);
    let sigkill_at = termination_grace_period_seconds;
    (sigterm_at, sigkill_at)
}

/// Convenience: build a PostStart Exec hook.
pub fn post_start_exec(cmd: Vec<String>) -> LifecycleHook {
    LifecycleHook {
        kind: HookKind::PostStart,
        handler: Handler::Exec(ExecHandler { command: cmd }),
    }
}

/// Convenience: build a PreStop HTTP-Get hook.
pub fn pre_stop_http_get(port: u16, path: &str) -> LifecycleHook {
    LifecycleHook {
        kind: HookKind::PreStop,
        handler: Handler::HttpGet(HttpGetHandler {
            scheme: HttpScheme::Http,
            host: None,
            port,
            path: path.to_string(),
            http_headers: vec![],
        }),
    }
}

/// Whether a PostStart handler failure should cause the container to be
/// killed (per upstream the PostStart hook is *required* — failure → kill +
/// FailedPostStartHook event). We capture this rule for callers.
pub fn post_start_failure_is_fatal() -> bool {
    true
}

/// PreStop failures are logged but never fatal (the container is going away
/// anyway). We capture this for symmetry.
pub fn pre_stop_failure_is_fatal() -> bool {
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    fn exec(cmd: &[&str]) -> Handler {
        Handler::Exec(ExecHandler {
            command: cmd.iter().map(|s| s.to_string()).collect(),
        })
    }
    fn http(port: u16, path: &str) -> Handler {
        Handler::HttpGet(HttpGetHandler {
            scheme: HttpScheme::Http,
            host: None,
            port,
            path: path.into(),
            http_headers: vec![],
        })
    }
    fn tcp(port: u16) -> Handler {
        Handler::TcpSocket(TcpSocketHandler { host: None, port })
    }
    fn sleep(s: i64) -> Handler {
        Handler::Sleep(SleepHandler { seconds: s })
    }

    #[test]
    fn validate_exec_requires_command() {
        assert!(validate_handler(&Handler::Exec(ExecHandler { command: vec![] })).is_err());
        validate_handler(&exec(&["sh", "-c", "echo"])).unwrap();
    }

    #[test]
    fn validate_http_rejects_zero_port() {
        assert!(validate_handler(&http(0, "/")).is_err());
        validate_handler(&http(80, "/healthz")).unwrap();
    }

    #[test]
    fn validate_http_rejects_empty_path() {
        let mut h = http(80, "");
        if let Handler::HttpGet(ref mut g) = h {
            g.path = "".into();
        }
        assert!(validate_handler(&h).is_err());
    }

    #[test]
    fn validate_http_rejects_invalid_header_name() {
        let h = Handler::HttpGet(HttpGetHandler {
            scheme: HttpScheme::Http,
            host: None,
            port: 80,
            path: "/".into(),
            http_headers: vec![("X Bad".into(), "v".into())],
        });
        assert!(validate_handler(&h).is_err());
    }

    #[test]
    fn validate_http_accepts_dash_underscore_alphanum_header() {
        let h = Handler::HttpGet(HttpGetHandler {
            scheme: HttpScheme::Http,
            host: None,
            port: 80,
            path: "/".into(),
            http_headers: vec![
                ("X-Custom".into(), "v".into()),
                ("X_TOKEN_2".into(), "v".into()),
            ],
        });
        validate_handler(&h).unwrap();
    }

    #[test]
    fn validate_tcp_rejects_zero_port() {
        assert!(validate_handler(&tcp(0)).is_err());
        validate_handler(&tcp(8080)).unwrap();
    }

    #[test]
    fn validate_sleep_rejects_negative_seconds() {
        assert!(validate_handler(&sleep(-1)).is_err());
        validate_handler(&sleep(0)).unwrap();
        validate_handler(&sleep(30)).unwrap();
    }

    #[test]
    fn is_blocking_classification() {
        assert!(is_blocking(&exec(&["sh"])));
        assert!(is_blocking(&sleep(5)));
        assert!(!is_blocking(&http(80, "/")));
        assert!(!is_blocking(&tcp(80)));
    }

    #[test]
    fn pre_stop_deadline_sleep_capped_at_grace() {
        // Sleep 10s, grace 30s → 10.
        assert_eq!(pre_stop_deadline_seconds(&sleep(10), 30), 10);
        // Sleep 60s, grace 30s → 30 (capped).
        assert_eq!(pre_stop_deadline_seconds(&sleep(60), 30), 30);
    }

    #[test]
    fn pre_stop_deadline_zero_grace_returns_zero() {
        assert_eq!(pre_stop_deadline_seconds(&sleep(10), 0), 0);
    }

    #[test]
    fn pre_stop_deadline_exec_returns_full_grace() {
        assert_eq!(pre_stop_deadline_seconds(&exec(&["sh"]), 45), 45);
    }

    #[test]
    fn termination_schedule_no_pre_stop() {
        let (term, kill) = termination_schedule(None, 30);
        assert_eq!(term, 0);
        assert_eq!(kill, 30);
    }

    #[test]
    fn termination_schedule_zero_grace_force_kills() {
        let (term, kill) = termination_schedule(Some(&exec(&["sh"])), 0);
        assert_eq!(term, 0);
        assert_eq!(kill, 0);
    }

    #[test]
    fn termination_schedule_negative_grace_force_kills() {
        let (term, kill) = termination_schedule(Some(&exec(&["sh"])), -5);
        assert_eq!(term, 0);
        assert_eq!(kill, 0);
    }

    #[test]
    fn termination_schedule_with_sleep_pre_stop() {
        let (term, kill) = termination_schedule(Some(&sleep(10)), 30);
        assert_eq!(term, 10);
        assert_eq!(kill, 30);
    }

    #[test]
    fn termination_schedule_caps_at_grace() {
        let (term, kill) = termination_schedule(Some(&sleep(60)), 30);
        assert_eq!(term, 30);
        assert_eq!(kill, 30);
    }

    #[test]
    fn termination_schedule_http_pre_stop_uses_full_grace() {
        let (term, kill) = termination_schedule(Some(&http(80, "/q")), 30);
        // HTTP doesn't bound itself; falls back to grace.
        assert_eq!(term, 30);
        assert_eq!(kill, 30);
    }

    #[test]
    fn post_start_exec_constructor() {
        let h = post_start_exec(vec!["sh".into(), "-c".into(), "true".into()]);
        assert_eq!(h.kind, HookKind::PostStart);
        assert!(matches!(h.handler, Handler::Exec(_)));
    }

    #[test]
    fn pre_stop_http_get_constructor() {
        let h = pre_stop_http_get(80, "/q");
        assert_eq!(h.kind, HookKind::PreStop);
        if let Handler::HttpGet(g) = h.handler {
            assert_eq!(g.port, 80);
            assert_eq!(g.path, "/q");
        } else {
            panic!();
        }
    }

    #[test]
    fn post_start_failure_kills_container() {
        assert!(post_start_failure_is_fatal());
    }

    #[test]
    fn pre_stop_failure_does_not_kill() {
        assert!(!pre_stop_failure_is_fatal());
    }

    #[test]
    fn handler_kinds_distinct() {
        assert_ne!(exec(&["a"]), http(80, "/"));
        assert_ne!(http(80, "/"), tcp(80));
        assert_ne!(tcp(80), sleep(0));
    }

    #[test]
    fn http_scheme_https_path() {
        let h = Handler::HttpGet(HttpGetHandler {
            scheme: HttpScheme::Https,
            host: Some("example".into()),
            port: 443,
            path: "/".into(),
            http_headers: vec![],
        });
        validate_handler(&h).unwrap();
    }

    #[test]
    fn pre_stop_deadline_exec_with_zero_grace() {
        assert_eq!(pre_stop_deadline_seconds(&exec(&["sh"]), 0), 0);
    }

    #[test]
    fn validate_handler_chain_all_ok() {
        for h in [
            exec(&["sh", "-c", "true"]),
            http(80, "/h"),
            tcp(8080),
            sleep(0),
            sleep(120),
        ] {
            validate_handler(&h).unwrap();
        }
    }

    #[test]
    fn header_with_empty_name_rejected() {
        let h = Handler::HttpGet(HttpGetHandler {
            scheme: HttpScheme::Http,
            host: None,
            port: 80,
            path: "/".into(),
            http_headers: vec![("".into(), "v".into())],
        });
        assert!(validate_handler(&h).is_err());
    }

    #[test]
    fn termination_schedule_pre_stop_sleep_zero() {
        let (term, kill) = termination_schedule(Some(&sleep(0)), 30);
        // 0-second sleep → SIGTERM at t=0, SIGKILL at grace.
        assert_eq!(term, 0);
        assert_eq!(kill, 30);
    }

    #[test]
    fn pre_stop_deadline_negative_grace_returns_zero() {
        // Defensive: negative grace from caller should not leak negative deadlines.
        assert_eq!(pre_stop_deadline_seconds(&sleep(5), -10), 0);
    }

    #[test]
    fn handler_serde_round_trip_exec() {
        let h = exec(&["sh", "-c", "true"]);
        let s = serde_json::to_string(&h).unwrap();
        let h2: Handler = serde_json::from_str(&s).unwrap();
        assert_eq!(h, h2);
    }

    #[test]
    fn handler_serde_round_trip_sleep() {
        let h = sleep(15);
        let s = serde_json::to_string(&h).unwrap();
        let h2: Handler = serde_json::from_str(&s).unwrap();
        assert_eq!(h, h2);
    }

    #[test]
    fn lifecycle_hook_kind_post_start_vs_pre_stop_distinct() {
        assert_ne!(HookKind::PostStart, HookKind::PreStop);
    }
}
