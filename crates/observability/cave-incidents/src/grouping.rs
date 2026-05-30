// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Alert grouping / deduplication engine.
//!
//! Faithful line-port of the incident-side grouping algorithm in grafana/oncall
//! v1.10.0:
//!   - `engine/apps/alerts/models/alert.py` :: `Alert.render_group_data` and
//!     `Alert.insert_random_uuid` — computes the `group_distinction` (an md5 hex
//!     digest of the grouping-id, or a random uuid when no grouping-id is set so
//!     the alert never groups; demo alerts likewise never group).
//!   - `engine/apps/alerts/models/alert_group.py` ::
//!     `AlertGroupQuerySet.get_or_create_grouping` — gets-or-creates the open
//!     incident group keyed by `(channel, distinction)`. When the channel allows
//!     source-based resolving and the incoming alert is a resolve signal, the
//!     alert re-attaches to the latest *resolved* group rather than opening a new
//!     one.
//!
//! Only the pure in-crate matcher is ported here. The Jinja `grouping_id`
//! templating, Slack rendering and DB optimistic-locking that surround it in
//! upstream remain sibling concerns (cave-portal / cave-net / persistence layer).

use uuid::Uuid;

// ── md5 (RFC 1321) ────────────────────────────────────────────────────────────
// hashlib.md5(...).hexdigest() in upstream. Implemented in-crate to keep the
// port self-contained (no new workspace dependency).

const MD5_S: [u32; 64] = [
    7, 12, 17, 22, 7, 12, 17, 22, 7, 12, 17, 22, 7, 12, 17, 22, 5, 9, 14, 20, 5, 9, 14, 20, 5, 9,
    14, 20, 5, 9, 14, 20, 4, 11, 16, 23, 4, 11, 16, 23, 4, 11, 16, 23, 4, 11, 16, 23, 6, 10, 15,
    21, 6, 10, 15, 21, 6, 10, 15, 21, 6, 10, 15, 21,
];

const MD5_K: [u32; 64] = [
    0xd76aa478, 0xe8c7b756, 0x242070db, 0xc1bdceee, 0xf57c0faf, 0x4787c62a, 0xa8304613, 0xfd469501,
    0x698098d8, 0x8b44f7af, 0xffff5bb1, 0x895cd7be, 0x6b901122, 0xfd987193, 0xa679438e, 0x49b40821,
    0xf61e2562, 0xc040b340, 0x265e5a51, 0xe9b6c7aa, 0xd62f105d, 0x02441453, 0xd8a1e681, 0xe7d3fbc8,
    0x21e1cde6, 0xc33707d6, 0xf4d50d87, 0x455a14ed, 0xa9e3e905, 0xfcefa3f8, 0x676f02d9, 0x8d2a4c8a,
    0xfffa3942, 0x8771f681, 0x6d9d6122, 0xfde5380c, 0xa4beea44, 0x4bdecfa9, 0xf6bb4b60, 0xbebfbc70,
    0x289b7ec6, 0xeaa127fa, 0xd4ef3085, 0x04881d05, 0xd9d4d039, 0xe6db99e5, 0x1fa27cf8, 0xc4ac5665,
    0xf4292244, 0x432aff97, 0xab9423a7, 0xfc93a039, 0x655b59c3, 0x8f0ccc92, 0xffeff47d, 0x85845dd1,
    0x6fa87e4f, 0xfe2ce6e0, 0xa3014314, 0x4e0811a1, 0xf7537e82, 0xbd3af235, 0x2ad7d2bb, 0xeb86d391,
];

/// Compute the lowercase-hex md5 digest of `data`, matching Python's
/// `hashlib.md5(data).hexdigest()`.
fn md5_hex(data: &[u8]) -> String {
    let (mut a0, mut b0, mut c0, mut d0) =
        (0x67452301u32, 0xefcdab89u32, 0x98badcfeu32, 0x10325476u32);

    // Pre-processing: append 0x80, pad with zeros to 56 mod 64, then 64-bit length.
    let bit_len = (data.len() as u64).wrapping_mul(8);
    let mut msg = data.to_vec();
    msg.push(0x80);
    while msg.len() % 64 != 56 {
        msg.push(0);
    }
    msg.extend_from_slice(&bit_len.to_le_bytes());

    for chunk in msg.chunks_exact(64) {
        let mut m = [0u32; 16];
        for (i, word) in m.iter_mut().enumerate() {
            *word = u32::from_le_bytes([
                chunk[i * 4],
                chunk[i * 4 + 1],
                chunk[i * 4 + 2],
                chunk[i * 4 + 3],
            ]);
        }

        let (mut a, mut b, mut c, mut d) = (a0, b0, c0, d0);
        for i in 0..64 {
            let (f, g) = match i {
                0..=15 => ((b & c) | (!b & d), i),
                16..=31 => ((d & b) | (!d & c), (5 * i + 1) % 16),
                32..=47 => (b ^ c ^ d, (3 * i + 5) % 16),
                _ => (c ^ (b | !d), (7 * i) % 16),
            };
            let f = f
                .wrapping_add(a)
                .wrapping_add(MD5_K[i])
                .wrapping_add(m[g]);
            a = d;
            d = c;
            c = b;
            b = b.wrapping_add(f.rotate_left(MD5_S[i]));
        }

        a0 = a0.wrapping_add(a);
        b0 = b0.wrapping_add(b);
        c0 = c0.wrapping_add(c);
        d0 = d0.wrapping_add(d);
    }

    let mut out = String::with_capacity(32);
    for word in [a0, b0, c0, d0] {
        for byte in word.to_le_bytes() {
            out.push_str(&format!("{byte:02x}"));
        }
    }
    out
}

// ── Grouping engine ───────────────────────────────────────────────────────────

/// An incoming alert, after the upstream Jinja templates have already been
/// evaluated into a (possibly absent) `grouping_id` and resolve signal.
#[derive(Debug, Clone)]
pub struct IncomingAlert {
    /// Receiving channel identifier — part of the grouping key.
    pub channel: String,
    /// The rendered `grouping_id` value (None when no grouping_id template).
    pub grouping_id: Option<String>,
    /// Whether the alert payload satisfied the resolve-condition template.
    pub is_resolve_signal: bool,
    /// Demo alerts must never group with anything.
    pub is_demo: bool,
}

/// Result of ingesting an alert into the grouping engine.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GroupingResult {
    /// The incident group the alert was attached to.
    pub group_id: Uuid,
    /// True if a brand-new group was opened for this alert.
    pub group_created: bool,
}

#[derive(Debug, Clone)]
struct Group {
    id: Uuid,
    channel: String,
    distinction: String,
    /// `is_open_for_grouping` in upstream — true while the group still accretes
    /// new alerts; cleared once the group is resolved.
    open_for_grouping: bool,
    resolved: bool,
    /// Monotonic sequence so we can pick the "latest" resolved group.
    seq: u64,
}

/// In-memory grouping engine — ports `AlertGroupQuerySet.get_or_create_grouping`.
#[derive(Debug, Default)]
pub struct GroupingEngine {
    groups: Vec<Group>,
    allow_source_based_resolving: bool,
    seq: u64,
}

impl GroupingEngine {
    pub fn new(allow_source_based_resolving: bool) -> Self {
        Self {
            groups: Vec::new(),
            allow_source_based_resolving,
            seq: 0,
        }
    }

    /// Port of `Alert.render_group_data` distinction logic + `insert_random_uuid`.
    ///
    /// `group_distinction = md5(grouping_id)` when a stable grouping-id is
    /// present and the alert is not a demo; otherwise a random uuid is mixed in
    /// (then md5'd) so the alert never groups with anything.
    pub fn group_distinction(grouping_id: Option<&str>, is_demo: bool) -> String {
        // Insert random uuid to prevent grouping of demo alerts or alerts with
        // group_distinction == None (upstream: insert_random_uuid).
        let distinction = match (grouping_id, is_demo) {
            (Some(id), false) => id.to_string(),
            (Some(id), true) => format!("{id}{}", Uuid::new_v4()),
            (None, _) => Uuid::new_v4().to_string(),
        };
        md5_hex(distinction.as_bytes())
    }

    /// Number of groups still open for grouping.
    pub fn open_group_count(&self) -> usize {
        self.groups.iter().filter(|g| g.open_for_grouping).count()
    }

    /// Mark a group resolved-by-source: it closes for grouping (upstream sets
    /// `is_open_for_grouping = None` on resolve).
    pub fn resolve_group_by_source(&mut self, group_id: Uuid) {
        if let Some(g) = self.groups.iter_mut().find(|g| g.id == group_id) {
            g.resolved = true;
            g.open_for_grouping = false;
        }
    }

    /// Port of `Alert.create` + `AlertGroupQuerySet.get_or_create_grouping`.
    pub fn ingest(&mut self, alert: IncomingAlert) -> GroupingResult {
        let distinction = Self::group_distinction(alert.grouping_id.as_deref(), alert.is_demo);

        // 1. Try to return the last open group for (channel, distinction).
        if let Some(g) = self
            .groups
            .iter()
            .rev()
            .find(|g| g.open_for_grouping && g.channel == alert.channel && g.distinction == distinction)
        {
            return GroupingResult {
                group_id: g.id,
                group_created: false,
            };
        }

        // 2. If it's an "OK" alert and the channel allows source-based resolving,
        //    re-attach to the latest resolved group with this distinction.
        if self.allow_source_based_resolving && alert.is_resolve_signal {
            if let Some(g) = self
                .groups
                .iter()
                .filter(|g| {
                    g.resolved && g.channel == alert.channel && g.distinction == distinction
                })
                .max_by_key(|g| g.seq)
            {
                return GroupingResult {
                    group_id: g.id,
                    group_created: false,
                };
            }
        }

        // 3. Otherwise open a brand new group.
        self.seq += 1;
        let group = Group {
            id: Uuid::new_v4(),
            channel: alert.channel,
            distinction,
            open_for_grouping: true,
            resolved: false,
            seq: self.seq,
        };
        let id = group.id;
        self.groups.push(group);
        GroupingResult {
            group_id: id,
            group_created: true,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn md5_known_vectors() {
        // RFC 1321 / standard md5 test vectors.
        assert_eq!(md5_hex(b""), "d41d8cd98f00b204e9800998ecf8427e");
        assert_eq!(md5_hex(b"abc"), "900150983cd24fb0d6963f7d28e17f72");
        assert_eq!(
            md5_hex(b"The quick brown fox jumps over the lazy dog"),
            "9e107d9d372bb6826bd81d3542a419d6"
        );
    }
}
