//! Channel + pattern fanout state machine — sans-io.
//!
//! Subscribers register as opaque `SubscriberId`s. Publishers push messages
//! into [`Broker::publish`], which returns the list of subscriber ids that
//! should receive the message. Pattern matching follows the Redis glob style:
//! `?` = single char, `*` = any sequence, `[abc]` = one of, `\` = escape.
//!
//! The broker is **not** RESP-aware; it deals only in routing decisions. The
//! caller is expected to encode `Push(message, channel, payload)` frames per
//! receiver via [`super::codec`].

use std::collections::{BTreeMap, BTreeSet};

pub type SubscriberId = u64;

#[derive(Debug, Default)]
pub struct Broker {
    /// channel → subscribers
    channels: BTreeMap<String, BTreeSet<SubscriberId>>,
    /// pattern → subscribers
    patterns: BTreeMap<String, BTreeSet<SubscriberId>>,
}

impl Broker {
    pub fn new() -> Self {
        Self::default()
    }

    /// SUBSCRIBE — returns `(channel, total_subs_for_subscriber)` per call.
    pub fn subscribe(&mut self, sub: SubscriberId, channel: &str) -> usize {
        self.channels
            .entry(channel.to_string())
            .or_default()
            .insert(sub);
        self.subscriber_count(sub)
    }

    /// UNSUBSCRIBE; if `channel` is `None`, drop all channel subscriptions.
    pub fn unsubscribe(&mut self, sub: SubscriberId, channel: Option<&str>) -> Vec<String> {
        let mut removed = Vec::new();
        match channel {
            Some(c) => {
                if let Some(set) = self.channels.get_mut(c) {
                    if set.remove(&sub) {
                        removed.push(c.to_string());
                    }
                    if set.is_empty() {
                        self.channels.remove(c);
                    }
                }
            }
            None => {
                let to_visit: Vec<String> = self.channels.keys().cloned().collect();
                for c in to_visit {
                    if let Some(set) = self.channels.get_mut(&c) {
                        if set.remove(&sub) {
                            removed.push(c.clone());
                        }
                        if set.is_empty() {
                            self.channels.remove(&c);
                        }
                    }
                }
            }
        }
        removed
    }

    pub fn psubscribe(&mut self, sub: SubscriberId, pattern: &str) -> usize {
        self.patterns
            .entry(pattern.to_string())
            .or_default()
            .insert(sub);
        self.subscriber_count(sub)
    }

    pub fn punsubscribe(&mut self, sub: SubscriberId, pattern: Option<&str>) -> Vec<String> {
        let mut removed = Vec::new();
        match pattern {
            Some(p) => {
                if let Some(set) = self.patterns.get_mut(p) {
                    if set.remove(&sub) {
                        removed.push(p.to_string());
                    }
                    if set.is_empty() {
                        self.patterns.remove(p);
                    }
                }
            }
            None => {
                let to_visit: Vec<String> = self.patterns.keys().cloned().collect();
                for p in to_visit {
                    if let Some(set) = self.patterns.get_mut(&p) {
                        if set.remove(&sub) {
                            removed.push(p.clone());
                        }
                        if set.is_empty() {
                            self.patterns.remove(&p);
                        }
                    }
                }
            }
        }
        removed
    }

    /// Returns the routing decision for one PUBLISH call:
    /// - `channel_subs`: direct subscribers of the channel.
    /// - `pattern_subs`: subscribers whose pattern matches, paired with the
    ///   matched pattern (Redis sends the pattern in pmessage frames).
    pub fn publish(&self, channel: &str) -> Routes {
        let channel_subs = self
            .channels
            .get(channel)
            .map(|s| s.iter().copied().collect())
            .unwrap_or_default();
        let mut pattern_subs = Vec::new();
        for (pat, subs) in &self.patterns {
            if glob_match(pat.as_bytes(), channel.as_bytes()) {
                for s in subs {
                    pattern_subs.push((pat.clone(), *s));
                }
            }
        }
        Routes {
            channel_subs,
            pattern_subs,
        }
    }

    /// Number of channel + pattern subscriptions held by `sub`.
    pub fn subscriber_count(&self, sub: SubscriberId) -> usize {
        let chans = self
            .channels
            .values()
            .filter(|s| s.contains(&sub))
            .count();
        let pats = self
            .patterns
            .values()
            .filter(|s| s.contains(&sub))
            .count();
        chans + pats
    }

    pub fn channel_count(&self) -> usize {
        self.channels.len()
    }

    pub fn pattern_count(&self) -> usize {
        self.patterns.len()
    }

    pub fn channels(&self) -> impl Iterator<Item = &str> {
        self.channels.keys().map(String::as_str)
    }
}

#[derive(Debug, PartialEq, Eq)]
pub struct Routes {
    pub channel_subs: Vec<SubscriberId>,
    pub pattern_subs: Vec<(String, SubscriberId)>,
}

impl Routes {
    pub fn total(&self) -> usize {
        self.channel_subs.len() + self.pattern_subs.len()
    }
}

/// Redis-style glob match. `\` escapes the next char.
pub fn glob_match(pattern: &[u8], target: &[u8]) -> bool {
    glob_match_inner(pattern, target)
}

fn glob_match_inner(mut pat: &[u8], mut tgt: &[u8]) -> bool {
    while !pat.is_empty() {
        match pat[0] {
            b'*' => {
                while !pat.is_empty() && pat[0] == b'*' {
                    pat = &pat[1..];
                }
                if pat.is_empty() {
                    return true;
                }
                for i in 0..=tgt.len() {
                    if glob_match_inner(pat, &tgt[i..]) {
                        return true;
                    }
                }
                return false;
            }
            b'?' => {
                if tgt.is_empty() {
                    return false;
                }
                pat = &pat[1..];
                tgt = &tgt[1..];
            }
            b'[' => {
                if tgt.is_empty() {
                    return false;
                }
                let mut negate = false;
                pat = &pat[1..];
                if !pat.is_empty() && pat[0] == b'^' {
                    negate = true;
                    pat = &pat[1..];
                }
                let mut matched = false;
                while !pat.is_empty() && pat[0] != b']' {
                    if pat.len() >= 3 && pat[1] == b'-' {
                        let lo = pat[0];
                        let hi = pat[2];
                        if tgt[0] >= lo && tgt[0] <= hi {
                            matched = true;
                        }
                        pat = &pat[3..];
                    } else {
                        if pat[0] == tgt[0] {
                            matched = true;
                        }
                        pat = &pat[1..];
                    }
                }
                if !pat.is_empty() && pat[0] == b']' {
                    pat = &pat[1..];
                }
                if matched == negate {
                    return false;
                }
                tgt = &tgt[1..];
            }
            b'\\' => {
                if pat.len() < 2 || tgt.is_empty() || pat[1] != tgt[0] {
                    return false;
                }
                pat = &pat[2..];
                tgt = &tgt[1..];
            }
            c => {
                if tgt.is_empty() || c != tgt[0] {
                    return false;
                }
                pat = &pat[1..];
                tgt = &tgt[1..];
            }
        }
    }
    tgt.is_empty()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn subscribe_returns_count_one() {
        let mut b = Broker::new();
        let n = b.subscribe(1, "news");
        assert_eq!(n, 1);
    }

    #[test]
    fn subscribe_two_channels_returns_two() {
        let mut b = Broker::new();
        b.subscribe(1, "news");
        let n = b.subscribe(1, "weather");
        assert_eq!(n, 2);
    }

    #[test]
    fn unsubscribe_specific_channel() {
        let mut b = Broker::new();
        b.subscribe(1, "a");
        b.subscribe(1, "b");
        let removed = b.unsubscribe(1, Some("a"));
        assert_eq!(removed, vec!["a"]);
        assert_eq!(b.subscriber_count(1), 1);
    }

    #[test]
    fn unsubscribe_all_returns_all_channels() {
        let mut b = Broker::new();
        b.subscribe(1, "x");
        b.subscribe(1, "y");
        let mut removed = b.unsubscribe(1, None);
        removed.sort();
        assert_eq!(removed, vec!["x", "y"]);
        assert_eq!(b.subscriber_count(1), 0);
    }

    #[test]
    fn publish_to_channel_with_two_subscribers() {
        let mut b = Broker::new();
        b.subscribe(1, "news");
        b.subscribe(2, "news");
        let r = b.publish("news");
        let mut subs = r.channel_subs.clone();
        subs.sort();
        assert_eq!(subs, vec![1, 2]);
        assert_eq!(r.pattern_subs, vec![]);
    }

    #[test]
    fn publish_to_empty_channel_yields_empty_routes() {
        let b = Broker::new();
        let r = b.publish("nothing");
        assert_eq!(r.total(), 0);
    }

    #[test]
    fn psubscribe_glob_star_matches() {
        let mut b = Broker::new();
        b.psubscribe(1, "news.*");
        let r = b.publish("news.weather");
        assert_eq!(r.pattern_subs, vec![("news.*".into(), 1)]);
    }

    #[test]
    fn psubscribe_question_mark_matches_one_char() {
        let mut b = Broker::new();
        b.psubscribe(1, "ch?n");
        let r = b.publish("chan");
        assert_eq!(r.pattern_subs.len(), 1);
        let r2 = b.publish("chinn");
        assert_eq!(r2.pattern_subs.len(), 0);
    }

    #[test]
    fn psubscribe_class_matches() {
        let mut b = Broker::new();
        b.psubscribe(1, "[ab]*");
        assert_eq!(b.publish("apple").pattern_subs.len(), 1);
        assert_eq!(b.publish("banana").pattern_subs.len(), 1);
        assert_eq!(b.publish("cherry").pattern_subs.len(), 0);
    }

    #[test]
    fn psubscribe_class_range_matches() {
        let mut b = Broker::new();
        b.psubscribe(1, "[a-c]");
        assert!(b.publish("a").pattern_subs.len() == 1);
        assert!(b.publish("b").pattern_subs.len() == 1);
        assert!(b.publish("c").pattern_subs.len() == 1);
        assert!(b.publish("d").pattern_subs.len() == 0);
    }

    #[test]
    fn psubscribe_class_negation() {
        let mut b = Broker::new();
        b.psubscribe(1, "[^a]");
        assert!(b.publish("a").pattern_subs.len() == 0);
        assert!(b.publish("b").pattern_subs.len() == 1);
    }

    #[test]
    fn psubscribe_escape_passes_literal() {
        let mut b = Broker::new();
        b.psubscribe(1, "a\\*b");
        assert!(b.publish("a*b").pattern_subs.len() == 1);
        assert!(b.publish("axb").pattern_subs.len() == 0);
    }

    #[test]
    fn punsubscribe_specific_pattern() {
        let mut b = Broker::new();
        b.psubscribe(1, "a*");
        b.psubscribe(1, "b*");
        let removed = b.punsubscribe(1, Some("a*"));
        assert_eq!(removed, vec!["a*"]);
        assert_eq!(b.subscriber_count(1), 1);
    }

    #[test]
    fn punsubscribe_all() {
        let mut b = Broker::new();
        b.psubscribe(1, "a*");
        b.psubscribe(1, "b*");
        let mut removed = b.punsubscribe(1, None);
        removed.sort();
        assert_eq!(removed, vec!["a*", "b*"]);
    }

    #[test]
    fn glob_match_empty_pattern_matches_only_empty() {
        assert!(glob_match(b"", b""));
        assert!(!glob_match(b"", b"x"));
    }

    #[test]
    fn glob_match_star_matches_anything() {
        assert!(glob_match(b"*", b""));
        assert!(glob_match(b"*", b"hello"));
    }

    #[test]
    fn glob_match_prefix_star() {
        assert!(glob_match(b"foo*", b"foobar"));
        assert!(!glob_match(b"foo*", b"barfoo"));
    }

    #[test]
    fn channel_count_tracks_distinct_channels() {
        let mut b = Broker::new();
        b.subscribe(1, "a");
        b.subscribe(2, "a");
        b.subscribe(3, "b");
        assert_eq!(b.channel_count(), 2);
    }

    #[test]
    fn subscriber_count_combines_channel_and_pattern() {
        let mut b = Broker::new();
        b.subscribe(1, "ch");
        b.psubscribe(1, "p*");
        assert_eq!(b.subscriber_count(1), 2);
    }

    #[test]
    fn publish_dedupes_through_pattern_match() {
        // direct + pattern both fire — Redis sends both messages and pmessages.
        let mut b = Broker::new();
        b.subscribe(1, "news");
        b.psubscribe(1, "ne*");
        let r = b.publish("news");
        assert_eq!(r.channel_subs, vec![1]);
        assert_eq!(r.pattern_subs, vec![("ne*".into(), 1)]);
    }
}
