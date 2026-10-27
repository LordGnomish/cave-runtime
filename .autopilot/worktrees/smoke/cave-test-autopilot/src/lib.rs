// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
/// Adds two integers together.
///
/// # Examples
///
/// ```
/// use cave_test_autopilot::add;
/// assert_eq!(add(2, 3), 5);
/// ```
pub fn add(a: i32, b: i32) -> i32 {
    a + b
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_add_positive() {
        assert_eq!(add(2, 3), 5);
    }

    #[test]
    fn test_add_negative() {
        assert_eq!(add(-1, -1), -2);
    }

    #[test]
    fn test_add_mixed() {
        assert_eq!(add(-5, 10), 5);
    }

    #[test]
    fn test_add_zeros() {
        assert_eq!(add(0, 0), 0);
    }
}