//! Cron-based schedule management and expression validation.

/// Validate a standard 5-field cron expression.
/// Fields: minute hour day-of-month month day-of-week
pub fn validate_cron_expression(expr: &str) -> bool {
    let parts: Vec<&str> = expr.split_whitespace().collect();
    if parts.len() != 5 {
        return false;
    }
    let ranges = [(0, 59), (0, 23), (1, 31), (1, 12), (0, 7)];
    parts.iter().zip(ranges.iter()).all(|(field, &(min, max))| {
        validate_cron_field(field, min, max)
    })
}

/// Validate a single cron field against [min, max].
pub fn validate_cron_field(field: &str, min: u32, max: u32) -> bool {
    if field == "*" {
        return true;
    }
    // Step: */n
    if let Some(step_str) = field.strip_prefix("*/") {
        return step_str
            .parse::<u32>()
            .map(|n| n > 0 && n <= max)
            .unwrap_or(false);
    }
    // Range: n-m
    if field.contains('-') {
        let parts: Vec<&str> = field.splitn(2, '-').collect();
        if parts.len() == 2 {
            if let (Ok(lo), Ok(hi)) = (parts[0].parse::<u32>(), parts[1].parse::<u32>()) {
                return lo >= min && hi <= max && lo <= hi;
            }
        }
        return false;
    }
    // List: n,m,...
    if field.contains(',') {
        return field
            .split(',')
            .all(|v| v.parse::<u32>().map(|n| n >= min && n <= max).unwrap_or(false));
    }
    // Single value
    field
        .parse::<u32>()
        .map(|n| n >= min && n <= max)
        .unwrap_or(false)
}

/// Describe when a cron expression next fires (human-readable stub).
pub fn describe_schedule(expr: &str) -> String {
    match expr {
        "0 * * * *" => "every hour".into(),
        "0 0 * * *" => "daily at midnight".into(),
        "0 0 * * 0" => "weekly on Sunday".into(),
        "0 0 1 * *" => "monthly on the 1st".into(),
        _ => format!("cron: {expr}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_validate_cron_standard_expressions() {
        assert!(validate_cron_expression("0 * * * *"));   // every hour
        assert!(validate_cron_expression("0 0 * * *"));   // daily midnight
        assert!(validate_cron_expression("*/15 * * * *")); // every 15 min
        assert!(validate_cron_expression("0 0 1 1 *"));   // Jan 1st
    }

    #[test]
    fn test_validate_cron_invalid_field_count() {
        assert!(!validate_cron_expression("* * * *"));      // 4 fields
        assert!(!validate_cron_expression("* * * * * *")); // 6 fields
        assert!(!validate_cron_expression(""));
    }

    #[test]
    fn test_validate_cron_field_star() {
        assert!(validate_cron_field("*", 0, 59));
    }

    #[test]
    fn test_validate_cron_field_step() {
        assert!(validate_cron_field("*/5", 0, 59));
        assert!(validate_cron_field("*/0", 0, 59) == false); // step=0 invalid
        assert!(validate_cron_field("*/60", 0, 59) == false); // > max
    }

    #[test]
    fn test_validate_cron_field_range() {
        assert!(validate_cron_field("1-5", 0, 59));
        assert!(!validate_cron_field("5-1", 0, 59)); // reversed
        assert!(!validate_cron_field("0-60", 0, 59)); // exceeds max
    }

    #[test]
    fn test_validate_cron_field_list() {
        assert!(validate_cron_field("1,15,30", 0, 59));
        assert!(!validate_cron_field("1,60,30", 0, 59)); // 60 out of range
    }

    #[test]
    fn test_validate_cron_field_single() {
        assert!(validate_cron_field("0", 0, 59));
        assert!(validate_cron_field("59", 0, 59));
        assert!(!validate_cron_field("60", 0, 59));
    }

    #[test]
    fn test_describe_schedule_known() {
        assert_eq!(describe_schedule("0 0 * * *"), "daily at midnight");
        assert_eq!(describe_schedule("0 * * * *"), "every hour");
    }

    #[test]
    fn test_describe_schedule_unknown() {
        let desc = describe_schedule("5 4 * * 1");
        assert!(desc.contains("cron:"));
    }
}
