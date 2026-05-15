//! Coverage report parsing — LCOV and Cobertura formats.

use std::collections::HashMap;

/// File-level coverage data.
#[derive(Debug, Clone)]
pub struct FileCoverage {
    pub path: String,
    pub total_lines: u64,
    pub covered_lines: u64,
}

/// Aggregate coverage report.
#[derive(Debug, Clone)]
pub struct CoverageReport {
    pub total_lines: u64,
    pub covered_lines: u64,
    pub coverage_percent: f64,
    pub files: Vec<FileCoverage>,
}

/// Parse LCOV (line coverage) format.
/// Format: TN:filename, LH:hit-lines, LF:total-lines, end_of_record
pub fn parse_lcov(content: &str) -> CoverageReport {
    let mut files: Vec<FileCoverage> = Vec::new();
    let mut current_file: Option<String> = None;
    let mut hit_lines = 0u64;
    let mut total_lines = 0u64;

    for line in content.lines() {
        if line.starts_with("TN:") {
            if let Some(path) = current_file.take() {
                files.push(FileCoverage {
                    path,
                    total_lines,
                    covered_lines: hit_lines,
                });
            }
            current_file = Some(line[3..].to_string());
            hit_lines = 0;
            total_lines = 0;
        } else if line.starts_with("LH:") {
            hit_lines = line[3..].parse().unwrap_or(0);
        } else if line.starts_with("LF:") {
            total_lines = line[3..].parse().unwrap_or(0);
        } else if line == "end_of_record" {
            if let Some(path) = current_file.take() {
                files.push(FileCoverage {
                    path,
                    total_lines,
                    covered_lines: hit_lines,
                });
            }
        }
    }

    // Finalize any remaining file
    if let Some(path) = current_file {
        files.push(FileCoverage {
            path,
            total_lines,
            covered_lines: hit_lines,
        });
    }

    let total_lines_all: u64 = files.iter().map(|f| f.total_lines).sum();
    let covered_lines_all: u64 = files.iter().map(|f| f.covered_lines).sum();
    let coverage_percent = if total_lines_all > 0 {
        (covered_lines_all as f64 / total_lines_all as f64) * 100.0
    } else {
        0.0
    };

    CoverageReport {
        total_lines: total_lines_all,
        covered_lines: covered_lines_all,
        coverage_percent,
        files,
    }
}

/// Parse Cobertura XML coverage format (basic implementation).
/// Extracts package/class/line coverage from complexity/covered attributes.
pub fn parse_cobertura(content: &str) -> CoverageReport {
    let mut files: Vec<FileCoverage> = Vec::new();
    let mut total_covered = 0u64;
    let mut total_lines = 0u64;

    // Simple regex-based parsing (not full XML parser)
    // Match: <package name="..." line-rate="0.85" complexity="..." />
    if let Ok(package_re) = regex::Regex::new(r#"<package\s+name="([^"]*)""#) {
        for cap in package_re.captures_iter(content) {
            if let Some(name) = cap.get(1) {
                // For simplicity, treat each package as one file entry
                // In production, would parse nested class/line elements
                files.push(FileCoverage {
                    path: name.as_str().to_string(),
                    total_lines: 0,
                    covered_lines: 0,
                });
            }
        }
    }

    // Try to extract overall line-rate attribute
    if let Ok(rate_re) = regex::Regex::new(r#"<coverage[^>]*line-rate="([^"]*)""#) {
        if let Some(cap) = rate_re.captures(content) {
            if let Some(rate_str) = cap.get(1) {
                if let Ok(rate) = rate_str.as_str().parse::<f64>() {
                    // Estimate based on rate: assume 1000 lines as default
                    let est_lines = 1000u64;
                    total_lines = est_lines;
                    total_covered = (est_lines as f64 * rate) as u64;
                }
            }
        }
    }

    let coverage_percent = if total_lines > 0 {
        (total_covered as f64 / total_lines as f64) * 100.0
    } else {
        0.0
    };

    CoverageReport {
        total_lines,
        covered_lines: total_covered,
        coverage_percent,
        files,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_lcov_basic() {
        let lcov = "TN:src/main.rs\nLF:100\nLH:85\nend_of_record\n";
        let report = parse_lcov(lcov);
        assert_eq!(report.total_lines, 100);
        assert_eq!(report.covered_lines, 85);
        assert!((report.coverage_percent - 85.0).abs() < 0.1);
    }

    #[test]
    fn test_parse_cobertura_basic() {
        let cobertura = r#"<coverage line-rate="0.80"><package name="com.example" /></coverage>"#;
        let report = parse_cobertura(cobertura);
        assert!((report.coverage_percent - 80.0).abs() < 0.1);
    }

    #[test]
    fn test_parse_lcov_empty() {
        let report = parse_lcov("");
        assert_eq!(report.total_lines, 0);
        assert_eq!(report.covered_lines, 0);
        assert_eq!(report.coverage_percent, 0.0);
        assert!(report.files.is_empty());
    }

    #[test]
    fn test_parse_lcov_multiple_files() {
        let lcov = "TN:src/a.rs\nLF:50\nLH:40\nend_of_record\nTN:src/b.rs\nLF:100\nLH:80\nend_of_record\n";
        let report = parse_lcov(lcov);
        assert_eq!(report.files.len(), 2);
        assert_eq!(report.total_lines, 150);
        assert_eq!(report.covered_lines, 120);
        assert!((report.coverage_percent - 80.0).abs() < 0.1);
    }

    #[test]
    fn test_parse_lcov_zero_lines() {
        let lcov = "TN:src/empty.rs\nLF:0\nLH:0\nend_of_record\n";
        let report = parse_lcov(lcov);
        assert_eq!(report.total_lines, 0);
        assert_eq!(report.coverage_percent, 0.0);
    }

    #[test]
    fn test_parse_lcov_no_end_of_record_finalizes() {
        // No trailing end_of_record; parser must still emit the file.
        let lcov = "TN:src/dangling.rs\nLF:30\nLH:15\n";
        let report = parse_lcov(lcov);
        assert_eq!(report.files.len(), 1);
        assert_eq!(report.files[0].path, "src/dangling.rs");
        assert_eq!(report.files[0].total_lines, 30);
        assert_eq!(report.files[0].covered_lines, 15);
    }

    #[test]
    fn test_parse_lcov_full_coverage() {
        let lcov = "TN:src/full.rs\nLF:42\nLH:42\nend_of_record\n";
        let report = parse_lcov(lcov);
        assert!((report.coverage_percent - 100.0).abs() < 0.01);
    }

    #[test]
    fn test_parse_lcov_zero_coverage() {
        let lcov = "TN:src/none.rs\nLF:50\nLH:0\nend_of_record\n";
        let report = parse_lcov(lcov);
        assert_eq!(report.coverage_percent, 0.0);
    }

    #[test]
    fn test_parse_lcov_garbage_lines_ignored() {
        let lcov = "DA:1,1\nBRDA:2,3,4,5\nFNDA:1,foo\nTN:src/clean.rs\nLF:10\nLH:7\nend_of_record\n";
        let report = parse_lcov(lcov);
        assert_eq!(report.files.len(), 1);
        assert_eq!(report.total_lines, 10);
        assert_eq!(report.covered_lines, 7);
    }

    #[test]
    fn test_parse_lcov_malformed_numbers_default_zero() {
        let lcov = "TN:src/bad.rs\nLF:not_a_number\nLH:also_bad\nend_of_record\n";
        let report = parse_lcov(lcov);
        assert_eq!(report.total_lines, 0);
        assert_eq!(report.covered_lines, 0);
    }

    #[test]
    fn test_parse_cobertura_full_rate() {
        let xml = r#"<coverage line-rate="1.0"><package name="full" /></coverage>"#;
        let report = parse_cobertura(xml);
        assert!((report.coverage_percent - 100.0).abs() < 0.01);
    }

    #[test]
    fn test_parse_cobertura_no_packages_no_rate() {
        let report = parse_cobertura("<coverage></coverage>");
        assert_eq!(report.total_lines, 0);
        assert_eq!(report.coverage_percent, 0.0);
        assert!(report.files.is_empty());
    }

    #[test]
    fn test_parse_cobertura_multiple_packages() {
        let xml = r#"<coverage line-rate="0.50"><package name="a"/><package name="b"/><package name="c"/></coverage>"#;
        let report = parse_cobertura(xml);
        assert_eq!(report.files.len(), 3);
        assert!((report.coverage_percent - 50.0).abs() < 0.1);
    }
}
