// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Integration tests for payroll computation engine.
//! These tests exercise cave_erp::payroll which does not yet exist —
//! they are written red-first per the project TDD protocol.

use cave_erp::payroll::{
    Allowance, AllowanceKind, Deduction, DeductionKind, PayslipLine, PayslipLineKind,
    compute_payslip, TaxBracket,
};
use uuid::Uuid;

#[test]
fn test_compute_payslip_basic_gross_equals_base_plus_allowances() {
    let employee_id = Uuid::new_v4();
    let base_salary = 5000.0_f64;

    let allowances = vec![
        Allowance { kind: AllowanceKind::Transport, amount: 200.0, currency: "EUR".to_string() },
        Allowance { kind: AllowanceKind::Meal, amount: 150.0, currency: "EUR".to_string() },
    ];
    let deductions: Vec<Deduction> = vec![];
    let brackets: Vec<TaxBracket> = vec![];

    let slip = compute_payslip(employee_id, base_salary, &allowances, &deductions, &brackets, "EUR");

    // gross = base + sum(allowances) = 5000 + 200 + 150 = 5350
    assert!((slip.gross - 5350.0).abs() < 0.01, "gross={}", slip.gross);
    // no deductions → net == gross
    assert!((slip.net - slip.gross).abs() < 0.01, "net={}", slip.net);
    assert_eq!(slip.employee_id, employee_id);
    assert_eq!(slip.currency, "EUR");
}

#[test]
fn test_compute_payslip_deductions_reduce_net() {
    let employee_id = Uuid::new_v4();
    let base_salary = 4000.0_f64;

    let allowances: Vec<Allowance> = vec![];
    let deductions = vec![
        Deduction { kind: DeductionKind::SocialSecurity, amount: 400.0, currency: "EUR".to_string() },
        Deduction { kind: DeductionKind::IncomeTax, amount: 600.0, currency: "EUR".to_string() },
    ];
    let brackets: Vec<TaxBracket> = vec![];

    let slip = compute_payslip(employee_id, base_salary, &allowances, &deductions, &brackets, "EUR");

    // gross = 4000
    assert!((slip.gross - 4000.0).abs() < 0.01);
    // net = gross - total_deductions = 4000 - 1000 = 3000
    assert!((slip.net - 3000.0).abs() < 0.01, "net={}", slip.net);
    assert_eq!(slip.deduction_total(), 1000.0_f64);
}

#[test]
fn test_compute_payslip_with_tax_brackets() {
    let employee_id = Uuid::new_v4();
    let base_salary = 3000.0_f64;

    let allowances: Vec<Allowance> = vec![];
    let deductions: Vec<Deduction> = vec![];
    // Progressive tax: 0–1000 @ 10%, 1000–2000 @ 20%, 2000+ @ 30%
    let brackets = vec![
        TaxBracket { from: 0.0, to: Some(1000.0), rate_pct: 10.0 },
        TaxBracket { from: 1000.0, to: Some(2000.0), rate_pct: 20.0 },
        TaxBracket { from: 2000.0, to: None, rate_pct: 30.0 },
    ];

    let slip = compute_payslip(employee_id, base_salary, &allowances, &deductions, &brackets, "EUR");

    // bracket tax: 1000*0.1 + 1000*0.2 + 1000*0.3 = 100+200+300 = 600
    let bracket_tax = slip.lines.iter()
        .filter(|l| l.kind == PayslipLineKind::BracketTax)
        .map(|l| l.amount)
        .sum::<f64>();
    assert!((bracket_tax - 600.0).abs() < 0.01, "bracket_tax={}", bracket_tax);
    // net = gross - bracket_tax = 3000 - 600 = 2400
    assert!((slip.net - 2400.0).abs() < 0.01, "net={}", slip.net);
}

#[test]
fn test_payslip_lines_are_complete() {
    let employee_id = Uuid::new_v4();
    let allowances = vec![
        Allowance { kind: AllowanceKind::Housing, amount: 300.0, currency: "EUR".to_string() },
    ];
    let deductions = vec![
        Deduction { kind: DeductionKind::Pension, amount: 150.0, currency: "EUR".to_string() },
    ];
    let brackets: Vec<TaxBracket> = vec![];

    let slip = compute_payslip(employee_id, 2000.0, &allowances, &deductions, &brackets, "EUR");

    // Must have: 1 base line + 1 allowance line + 1 deduction line
    let base_lines = slip.lines.iter().filter(|l| l.kind == PayslipLineKind::Base).count();
    let allow_lines = slip.lines.iter().filter(|l| l.kind == PayslipLineKind::Allowance).count();
    let ded_lines = slip.lines.iter().filter(|l| l.kind == PayslipLineKind::Deduction).count();

    assert_eq!(base_lines, 1);
    assert_eq!(allow_lines, 1);
    assert_eq!(ded_lines, 1);
}
