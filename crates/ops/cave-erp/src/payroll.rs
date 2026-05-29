// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Payroll computation engine.
//!
//! Computes a payslip for an employee given a base salary, allowances,
//! explicit deductions, and an optional progressive income-tax bracket table.
//! Mirrors the payroll-processing surface of ERPNext's HRM / Payroll module.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

// ===== Tax bracket =====

/// A single progressive-tax bracket: income in [from, to) is taxed at `rate_pct`.
/// `to: None` means the bracket is open-ended (top bracket).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaxBracket {
    pub from: f64,
    pub to: Option<f64>,
    /// Rate as a percentage (e.g. 20.0 means 20 %)
    pub rate_pct: f64,
}

// ===== Allowances =====

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum AllowanceKind {
    Transport,
    Meal,
    Housing,
    Medical,
    Other(String),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Allowance {
    pub kind: AllowanceKind,
    pub amount: f64,
    pub currency: String,
}

// ===== Deductions =====

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum DeductionKind {
    SocialSecurity,
    IncomeTax,
    Pension,
    UnionFee,
    Advance,
    Other(String),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Deduction {
    pub kind: DeductionKind,
    pub amount: f64,
    pub currency: String,
}

// ===== Payslip output =====

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum PayslipLineKind {
    Base,
    Allowance,
    Deduction,
    BracketTax,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PayslipLine {
    pub kind: PayslipLineKind,
    pub description: String,
    pub amount: f64,
}

/// A computed payslip for a single employee for a single period.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Payslip {
    pub id: Uuid,
    pub employee_id: Uuid,
    pub currency: String,
    /// Base salary + all allowances (before any deductions / taxes)
    pub gross: f64,
    /// Total deductions (explicit + bracket tax)
    pub total_deductions: f64,
    /// Take-home pay = gross − total_deductions
    pub net: f64,
    /// Itemised breakdown (one line per allowance, deduction, and bracket-tax)
    pub lines: Vec<PayslipLine>,
}

impl Payslip {
    /// Convenience: sum of explicit Deduction-kind lines only (not bracket tax).
    pub fn deduction_total(&self) -> f64 {
        self.lines
            .iter()
            .filter(|l| l.kind == PayslipLineKind::Deduction)
            .map(|l| l.amount)
            .sum()
    }
}

// ===== Computation =====

/// Compute a payslip.
///
/// Algorithm:
/// 1. gross = base_salary + sum(allowances)
/// 2. Apply explicit deductions (fixed amounts).
/// 3. Compute progressive bracket tax on gross (if brackets are provided).
/// 4. net = gross − total_explicit_deductions − bracket_tax
pub fn compute_payslip(
    employee_id: Uuid,
    base_salary: f64,
    allowances: &[Allowance],
    deductions: &[Deduction],
    brackets: &[TaxBracket],
    currency: &str,
) -> Payslip {
    let mut lines: Vec<PayslipLine> = Vec::new();

    // Line: base salary
    lines.push(PayslipLine {
        kind: PayslipLineKind::Base,
        description: "Base Salary".to_string(),
        amount: base_salary,
    });

    // Lines: allowances
    let allowance_total: f64 = allowances.iter().map(|a| a.amount).sum();
    for a in allowances {
        lines.push(PayslipLine {
            kind: PayslipLineKind::Allowance,
            description: format!("{:?} Allowance", a.kind),
            amount: a.amount,
        });
    }

    let gross = base_salary + allowance_total;

    // Lines: explicit deductions
    let explicit_deduction_total: f64 = deductions.iter().map(|d| d.amount).sum();
    for d in deductions {
        lines.push(PayslipLine {
            kind: PayslipLineKind::Deduction,
            description: format!("{:?}", d.kind),
            amount: d.amount,
        });
    }

    // Lines: progressive bracket tax on gross
    let bracket_tax = compute_bracket_tax(gross, brackets, &mut lines);

    let total_deductions = explicit_deduction_total + bracket_tax;
    let net = (gross - total_deductions).max(0.0);

    Payslip {
        id: Uuid::new_v4(),
        employee_id,
        currency: currency.to_string(),
        gross,
        total_deductions,
        net,
        lines,
    }
}

/// Compute progressive tax from a bracket table applied to `taxable_income`.
/// Appends one `PayslipLine::BracketTax` per bracket that applies.
/// Returns the total bracket tax amount.
fn compute_bracket_tax(taxable_income: f64, brackets: &[TaxBracket], lines: &mut Vec<PayslipLine>) -> f64 {
    let mut total = 0.0;
    for bracket in brackets {
        let lower = bracket.from;
        let upper = bracket.to.unwrap_or(f64::MAX);

        if taxable_income <= lower {
            break;
        }

        let applicable_income = taxable_income.min(upper) - lower;
        if applicable_income <= 0.0 {
            continue;
        }

        let tax = applicable_income * bracket.rate_pct / 100.0;
        total += tax;

        lines.push(PayslipLine {
            kind: PayslipLineKind::BracketTax,
            description: format!(
                "Income Tax {:.0}–{} @ {:.1}%",
                lower,
                bracket.to.map(|t| format!("{:.0}", t)).unwrap_or_else(|| "∞".to_string()),
                bracket.rate_pct,
            ),
            amount: tax,
        });
    }
    total
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_no_allowances_no_deductions_gross_equals_base() {
        let slip = compute_payslip(Uuid::new_v4(), 3000.0, &[], &[], &[], "EUR");
        assert!((slip.gross - 3000.0).abs() < 0.01);
        assert!((slip.net - 3000.0).abs() < 0.01);
    }

    #[test]
    fn test_bracket_tax_open_top() {
        let brackets = vec![TaxBracket { from: 0.0, to: None, rate_pct: 15.0 }];
        let slip = compute_payslip(Uuid::new_v4(), 2000.0, &[], &[], &brackets, "EUR");
        // 2000 * 0.15 = 300
        assert!((slip.total_deductions - 300.0).abs() < 0.01);
        assert!((slip.net - 1700.0).abs() < 0.01);
    }

    #[test]
    fn test_net_never_negative() {
        let deductions = vec![
            Deduction { kind: DeductionKind::IncomeTax, amount: 9999.0, currency: "EUR".to_string() },
        ];
        let slip = compute_payslip(Uuid::new_v4(), 500.0, &[], &deductions, &[], "EUR");
        assert!(slip.net >= 0.0);
    }
}
