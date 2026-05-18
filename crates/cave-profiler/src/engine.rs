// SPDX-License-Identifier: AGPL-3.0-or-later
use crate::models::{ProfileSession, StackFrame};

pub fn top_functions<'a>(frames: &'a [StackFrame], n: usize) -> Vec<&'a StackFrame> {
    let mut sorted: Vec<&StackFrame> = frames.iter().collect();
    sorted.sort_by(|a, b| b.self_samples.cmp(&a.self_samples));
    sorted.truncate(n);
    sorted
}

pub fn session_duration_secs(session: &ProfileSession) -> Option<i64> {
    session
        .ended_at
        .map(|end| (end - session.started_at).num_seconds())
}

pub fn samples_per_second(session: &ProfileSession) -> Option<f64> {
    session_duration_secs(session).map(|d| {
        if d == 0 {
            0.0
        } else {
            session.samples as f64 / d as f64
        }
    })
}

pub fn find_hotspot<'a>(frames: &'a [StackFrame]) -> Option<&'a StackFrame> {
    frames.iter().max_by_key(|f| f.self_samples)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{ProfileSession, ProfileType, StackFrame};
    use chrono::{Duration, Utc};
    use uuid::Uuid;

    fn make_frame(function: &str, self_samples: u64) -> StackFrame {
        StackFrame {
            function: function.to_string(),
            file: "main.rs".to_string(),
            line: 42,
            self_samples,
            cumulative_samples: self_samples + 10,
        }
    }

    fn make_session(samples: u64, duration_secs: Option<i64>) -> ProfileSession {
        let started_at = Utc::now();
        let ended_at = duration_secs.map(|d| started_at + Duration::seconds(d));
        ProfileSession {
            id: Uuid::new_v4(),
            service: "api".to_string(),
            profile_type: ProfileType::Cpu,
            started_at,
            ended_at,
            samples,
            frames: vec![],
        }
    }

    #[test]
    fn test_top_functions_sorted() {
        let frames = vec![
            make_frame("foo", 10),
            make_frame("bar", 50),
            make_frame("baz", 30),
        ];
        let top = top_functions(&frames, 2);
        assert_eq!(top.len(), 2);
        assert_eq!(top[0].function, "bar");
        assert_eq!(top[1].function, "baz");
    }

    #[test]
    fn test_top_functions_fewer_than_n() {
        let frames = vec![make_frame("foo", 10), make_frame("bar", 50)];
        let top = top_functions(&frames, 5);
        assert_eq!(top.len(), 2);
    }

    #[test]
    fn test_session_duration_secs_none_for_running() {
        let session = make_session(1000, None);
        assert_eq!(session_duration_secs(&session), None);
    }

    #[test]
    fn test_find_hotspot() {
        let frames = vec![
            make_frame("foo", 10),
            make_frame("hotfn", 200),
            make_frame("bar", 50),
        ];
        let hotspot = find_hotspot(&frames).unwrap();
        assert_eq!(hotspot.function, "hotfn");
    }

    #[test]
    fn test_find_hotspot_empty() {
        let frames: Vec<StackFrame> = vec![];
        assert!(find_hotspot(&frames).is_none());
    }
}
