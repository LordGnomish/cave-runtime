// SPDX-License-Identifier: AGPL-3.0-or-later
//! Badges plugin — small SVG-style status badges per service.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Badge {
    pub label: String,
    pub message: String,
    pub color: BadgeColor,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BadgeColor {
    BrightGreen,
    Green,
    YellowGreen,
    Yellow,
    Orange,
    Red,
    Lightgrey,
    Blue,
}

impl BadgeColor {
    pub fn css(&self) -> &'static str {
        match self {
            BadgeColor::BrightGreen => "#4c1",
            BadgeColor::Green => "#97ca00",
            BadgeColor::YellowGreen => "#a4a61d",
            BadgeColor::Yellow => "#dfb317",
            BadgeColor::Orange => "#fe7d37",
            BadgeColor::Red => "#e05d44",
            BadgeColor::Lightgrey => "#9f9f9f",
            BadgeColor::Blue => "#007ec6",
        }
    }
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum BadgeError {
    #[error("invalid label")]
    InvalidLabel,
    #[error("invalid message")]
    InvalidMessage,
}

pub fn make_badge(label: &str, message: &str, color: BadgeColor) -> Result<Badge, BadgeError> {
    if label.is_empty() || label.len() > 64 {
        return Err(BadgeError::InvalidLabel);
    }
    if message.is_empty() || message.len() > 64 {
        return Err(BadgeError::InvalidMessage);
    }
    if label.contains('<') || message.contains('<') {
        return Err(BadgeError::InvalidLabel);
    }
    Ok(Badge {
        label: label.into(),
        message: message.into(),
        color,
    })
}

pub fn coverage_badge(pct: u8) -> Badge {
    let color = match pct {
        0..=49 => BadgeColor::Red,
        50..=69 => BadgeColor::Orange,
        70..=79 => BadgeColor::Yellow,
        80..=89 => BadgeColor::YellowGreen,
        90..=94 => BadgeColor::Green,
        _ => BadgeColor::BrightGreen,
    };
    Badge {
        label: "coverage".into(),
        message: format!("{pct}%"),
        color,
    }
}

pub fn build_badge(succeeded: bool) -> Badge {
    if succeeded {
        Badge {
            label: "build".into(),
            message: "passing".into(),
            color: BadgeColor::BrightGreen,
        }
    } else {
        Badge {
            label: "build".into(),
            message: "failing".into(),
            color: BadgeColor::Red,
        }
    }
}

pub fn version_badge(version: &str) -> Result<Badge, BadgeError> {
    make_badge("version", version, BadgeColor::Blue)
}

pub fn render_svg(b: &Badge) -> String {
    let label = escape_svg(&b.label);
    let message = escape_svg(&b.message);
    let color = b.color.css();
    let lw = 6 * b.label.len() + 10;
    let mw = 6 * b.message.len() + 10;
    let total = lw + mw;
    format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{total}\" height=\"20\">\
         <rect x=\"0\" y=\"0\" width=\"{lw}\" height=\"20\" fill=\"#555\"/>\
         <rect x=\"{lw}\" y=\"0\" width=\"{mw}\" height=\"20\" fill=\"{color}\"/>\
         <text x=\"5\" y=\"14\" fill=\"#fff\">{label}</text>\
         <text x=\"{tx}\" y=\"14\" fill=\"#fff\">{message}</text>\
         </svg>",
        tx = lw + 5
    )
}

fn escape_svg(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        match ch {
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '&' => out.push_str("&amp;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            c => out.push(c),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn make_badge_basic() {
        let b = make_badge("build", "passing", BadgeColor::BrightGreen).unwrap();
        assert_eq!(b.label, "build");
        assert_eq!(b.message, "passing");
    }

    #[test]
    fn make_badge_empty_label_rejected() {
        let err = make_badge("", "msg", BadgeColor::Blue).unwrap_err();
        assert_eq!(err, BadgeError::InvalidLabel);
    }

    #[test]
    fn make_badge_long_label_rejected() {
        let l = "a".repeat(65);
        let err = make_badge(&l, "m", BadgeColor::Blue).unwrap_err();
        assert_eq!(err, BadgeError::InvalidLabel);
    }

    #[test]
    fn make_badge_html_in_label_rejected() {
        let err = make_badge("a<b", "m", BadgeColor::Blue).unwrap_err();
        assert_eq!(err, BadgeError::InvalidLabel);
    }

    #[test]
    fn make_badge_empty_message_rejected() {
        let err = make_badge("L", "", BadgeColor::Blue).unwrap_err();
        assert_eq!(err, BadgeError::InvalidMessage);
    }

    #[test]
    fn make_badge_long_message_rejected() {
        let m = "a".repeat(65);
        let err = make_badge("L", &m, BadgeColor::Blue).unwrap_err();
        assert_eq!(err, BadgeError::InvalidMessage);
    }

    #[test]
    fn coverage_badge_red_below_50() {
        let b = coverage_badge(49);
        assert_eq!(b.color, BadgeColor::Red);
    }

    #[test]
    fn coverage_badge_orange_50_to_69() {
        for pct in [50, 65, 69] {
            assert_eq!(coverage_badge(pct).color, BadgeColor::Orange);
        }
    }

    #[test]
    fn coverage_badge_yellow_70_to_79() {
        assert_eq!(coverage_badge(75).color, BadgeColor::Yellow);
    }

    #[test]
    fn coverage_badge_yellowgreen_80_to_89() {
        assert_eq!(coverage_badge(85).color, BadgeColor::YellowGreen);
    }

    #[test]
    fn coverage_badge_green_90_to_94() {
        assert_eq!(coverage_badge(92).color, BadgeColor::Green);
    }

    #[test]
    fn coverage_badge_bright_green_95_plus() {
        assert_eq!(coverage_badge(99).color, BadgeColor::BrightGreen);
        assert_eq!(coverage_badge(100).color, BadgeColor::BrightGreen);
    }

    #[test]
    fn coverage_badge_message_includes_pct() {
        let b = coverage_badge(73);
        assert_eq!(b.message, "73%");
    }

    #[test]
    fn build_badge_passing() {
        let b = build_badge(true);
        assert_eq!(b.message, "passing");
        assert_eq!(b.color, BadgeColor::BrightGreen);
    }

    #[test]
    fn build_badge_failing() {
        let b = build_badge(false);
        assert_eq!(b.message, "failing");
        assert_eq!(b.color, BadgeColor::Red);
    }

    #[test]
    fn version_badge_blue() {
        let b = version_badge("1.2.3").unwrap();
        assert_eq!(b.color, BadgeColor::Blue);
        assert_eq!(b.message, "1.2.3");
    }

    #[test]
    fn render_svg_contains_label_and_message() {
        let b = Badge {
            label: "build".into(),
            message: "passing".into(),
            color: BadgeColor::BrightGreen,
        };
        let svg = render_svg(&b);
        assert!(svg.contains("build"));
        assert!(svg.contains("passing"));
        assert!(svg.starts_with("<svg"));
    }

    #[test]
    fn render_svg_uses_color_hex() {
        let b = Badge {
            label: "x".into(),
            message: "y".into(),
            color: BadgeColor::Red,
        };
        let svg = render_svg(&b);
        assert!(svg.contains("#e05d44"));
    }

    #[test]
    fn render_svg_escapes_html_in_label() {
        let b = Badge {
            label: "<bad>".into(),
            message: "&x".into(),
            color: BadgeColor::Blue,
        };
        let svg = render_svg(&b);
        assert!(!svg.contains("<bad>"));
        assert!(svg.contains("&lt;bad&gt;"));
        assert!(svg.contains("&amp;x"));
    }

    #[test]
    fn badge_color_css_distinct() {
        let mut seen = std::collections::HashSet::new();
        for c in [
            BadgeColor::BrightGreen, BadgeColor::Green, BadgeColor::YellowGreen,
            BadgeColor::Yellow, BadgeColor::Orange, BadgeColor::Red,
            BadgeColor::Lightgrey, BadgeColor::Blue,
        ] {
            assert!(seen.insert(c.css()));
        }
    }

    #[test]
    fn badge_serializes() {
        let b = Badge { label: "L".into(), message: "M".into(), color: BadgeColor::Green };
        let s = serde_json::to_string(&b).unwrap();
        assert!(s.contains("\"color\":\"green\""));
    }
}
