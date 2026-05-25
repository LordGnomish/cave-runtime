// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Fuzzy filter for the TUI item list.
//!
//! Subsequence match (each byte of `query` must appear in `target` in
//! order) plus a small score that favours consecutive runs and
//! prefix/word-start matches. Inspired by `fzf`'s scoring without
//! pulling the dependency.

/// Returns true if `query` is a (case-insensitive) subsequence of `target`.
pub fn fuzzy_match(target: &str, query: &str) -> bool {
    if query.is_empty() {
        return true;
    }
    let target = target.to_lowercase();
    let query = query.to_lowercase();
    let mut q = query.chars();
    let mut needle = match q.next() {
        Some(c) => c,
        None => return true,
    };
    for ch in target.chars() {
        if ch == needle {
            match q.next() {
                Some(c) => needle = c,
                None => return true,
            }
        }
    }
    false
}

/// Score a fuzzy match. Higher is better. Returns `None` if no match.
pub fn fuzzy_score(target: &str, query: &str) -> Option<i32> {
    if query.is_empty() {
        return Some(0);
    }
    if !fuzzy_match(target, query) {
        return None;
    }
    let target_lower = target.to_lowercase();
    let query_lower = query.to_lowercase();

    let mut score: i32 = 0;
    let mut last_match: Option<usize> = None;
    let mut q_chars = query_lower.chars();
    let mut needle = q_chars.next();

    for (i, ch) in target_lower.chars().enumerate() {
        if Some(ch) == needle {
            score += 10;
            if let Some(prev) = last_match {
                if prev + 1 == i {
                    score += 5; // consecutive bonus
                }
            }
            if i == 0 {
                score += 8; // prefix bonus
            }
            // word-start bonus
            if i > 0 {
                let prev_ch = target_lower.chars().nth(i - 1).unwrap_or(' ');
                if prev_ch == ' ' || prev_ch == '-' || prev_ch == '_' || prev_ch == '/' {
                    score += 3;
                }
            }
            last_match = Some(i);
            needle = q_chars.next();
            if needle.is_none() {
                break;
            }
        }
    }
    // Penalise targets that are much longer than the query.
    score -= (target.len() as i32 - query.len() as i32).max(0);
    Some(score)
}

/// Filter and rank `items` by `query`. Stable sort: ties keep original order.
pub fn filter_rank<S: AsRef<str> + Clone>(items: &[S], query: &str) -> Vec<S> {
    let mut scored: Vec<(usize, S, i32)> = items
        .iter()
        .enumerate()
        .filter_map(|(i, s)| fuzzy_score(s.as_ref(), query).map(|sc| (i, s.clone(), sc)))
        .collect();
    scored.sort_by(|a, b| b.2.cmp(&a.2).then_with(|| a.0.cmp(&b.0)));
    scored.into_iter().map(|(_, s, _)| s).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_query_matches_everything() {
        assert!(fuzzy_match("anything", ""));
        assert!(fuzzy_match("", ""));
    }

    #[test]
    fn substring_matches() {
        assert!(fuzzy_match("nginx", "ngx"));
    }

    #[test]
    fn subsequence_matches() {
        assert!(fuzzy_match("cave-apiserver", "csv"));
    }

    #[test]
    fn case_insensitive() {
        assert!(fuzzy_match("Nginx", "ngx"));
    }

    #[test]
    fn no_match() {
        assert!(!fuzzy_match("nginx", "redis"));
    }

    #[test]
    fn order_matters() {
        assert!(fuzzy_match("redis", "rds"));
        assert!(!fuzzy_match("redis", "sdr"));
    }

    #[test]
    fn empty_target_no_match_for_nonempty_query() {
        assert!(!fuzzy_match("", "x"));
    }

    #[test]
    fn score_empty_query_zero() {
        assert_eq!(fuzzy_score("anything", ""), Some(0));
    }

    #[test]
    fn score_no_match_none() {
        assert_eq!(fuzzy_score("nginx", "redis"), None);
    }

    #[test]
    fn score_prefix_higher_than_middle() {
        let prefix = fuzzy_score("api-server", "api").unwrap();
        let middle = fuzzy_score("cave-api-server", "api").unwrap();
        assert!(prefix > middle);
    }

    #[test]
    fn score_consecutive_higher_than_spread() {
        let consec = fuzzy_score("nginx-pod", "ngx").unwrap();
        let spread = fuzzy_score("n-something-g-something-x", "ngx").unwrap();
        assert!(consec > spread);
    }

    #[test]
    fn score_word_start_bonus() {
        // The matcher is greedy (first-fit). To exercise the
        // word-start bonus we need a target where the first `a` sits
        // right after a separator. `x-api` has it (prev_ch = `-`),
        // `xapi` doesn't (prev_ch = `x`). Same length so the length
        // penalty doesn't dominate the comparison.
        let word_start = fuzzy_score("x-api", "a").unwrap();
        let middle = fuzzy_score("xappp", "a").unwrap();
        assert!(
            word_start > middle,
            "word-start match should beat in-word match (got {} vs {})",
            word_start,
            middle
        );
    }

    #[test]
    fn filter_rank_orders_by_score() {
        let items = vec!["nginx", "ngx-app", "another"];
        let ranked = filter_rank(&items, "ngx");
        // "ngx-app" scores higher (prefix + consecutive).
        assert_eq!(ranked[0], "ngx-app");
    }

    #[test]
    fn filter_rank_drops_non_matching() {
        let items = vec!["nginx", "redis", "postgres"];
        let ranked = filter_rank(&items, "ng");
        assert_eq!(ranked, vec!["nginx"]);
    }

    #[test]
    fn filter_rank_empty_query_keeps_order() {
        let items = vec!["a", "b", "c"];
        let ranked = filter_rank(&items, "");
        assert_eq!(ranked, vec!["a", "b", "c"]);
    }

    #[test]
    fn filter_rank_stable_for_ties() {
        // Both "ab" and "abc" score similarly for query "a"; original
        // order should be preserved.
        let items = vec!["ab", "abc"];
        let ranked = filter_rank(&items, "a");
        assert_eq!(ranked, vec!["ab", "abc"]);
    }

    #[test]
    fn matches_special_chars_in_target() {
        assert!(fuzzy_match("cave-api-server-v2", "csv"));
    }

    #[test]
    fn score_includes_length_penalty() {
        let short = fuzzy_score("api", "api").unwrap();
        let long = fuzzy_score("api-server-much-longer", "api").unwrap();
        assert!(short > long);
    }

    #[test]
    fn full_match_scores_higher_than_partial() {
        let full = fuzzy_score("api", "api").unwrap();
        let partial = fuzzy_score("apixx", "api").unwrap();
        assert!(full >= partial);
    }
}
