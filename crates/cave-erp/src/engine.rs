use crate::models::*;
use std::collections::HashMap;
use uuid::Uuid;

/// Calculate line subtotal: (qty * unit_price * (1 - discount%)) * (1 + tax%)
pub fn line_subtotal(quantity: f64, unit_price: f64, discount_pct: f64, tax_pcts: &[f64]) -> f64 {
    let after_discount = quantity * unit_price * (1.0 - (discount_pct / 100.0));
    let total_tax_pct = tax_pcts.iter().sum::<f64>();
    after_discount * (1.0 + (total_tax_pct / 100.0))
}

/// Sum all line subtotals
pub fn doc_total(lines_subtotals: &[f64]) -> f64 {
    lines_subtotals.iter().sum()
}

/// Aggregate on-hand quantities by location from done stock moves
pub fn on_hand_by_location(
    moves: &[StockMove],
    product_id: Uuid,
) -> HashMap<Uuid, f64> {
    let mut result: HashMap<Uuid, f64> = HashMap::new();

    for m in moves {
        if m.product_id != product_id || m.state != StockMoveState::Done {
            continue;
        }

        // Outbound: decrement
        *result.entry(m.from_location_id).or_insert(0.0) -= m.qty;

        // Inbound: increment
        *result.entry(m.to_location_id).or_insert(0.0) += m.qty;
    }

    // Remove zero/negative entries
    result.retain(|_, v| *v > 0.0);
    result
}

/// Multi-level BOM explosion with recursive lookup
pub fn explode_bom(
    bom: &Bom,
    boms: &HashMap<Uuid, Bom>,
    target_qty: f64,
) -> Vec<(Uuid, f64)> {
    let mut result: HashMap<Uuid, f64> = HashMap::new();

    fn recurse(
        bom: &Bom,
        qty_needed: f64,
        boms: &HashMap<Uuid, Bom>,
        result: &mut HashMap<Uuid, f64>,
    ) {
        for comp in &bom.components {
            let comp_qty = qty_needed * comp.qty / bom.quantity;
            *result.entry(comp.product_id).or_insert(0.0) += comp_qty;

            // Try to recursively explode if this component is also a BOM product
            if let Some(sub_bom) = boms.get(&comp.product_id) {
                recurse(sub_bom, comp_qty, boms, result);
            }
        }
    }

    recurse(bom, target_qty, boms, &mut result);

    let mut flattened: Vec<_> = result.into_iter().collect();
    flattened.sort_by_key(|k| k.0);
    flattened
}

/// Generic pipeline stage advancement trait and function
pub trait Stage {
    fn id(&self) -> Uuid;
    fn order(&self) -> u32;
}

impl Stage for CrmStage {
    fn id(&self) -> Uuid {
        self.id
    }
    fn order(&self) -> u32 {
        self.order
    }
}

impl Stage for RecruitmentStage {
    fn id(&self) -> Uuid {
        self.id
    }
    fn order(&self) -> u32 {
        self.order
    }
}

/// Advance from current stage to next stage (by `order`). Returns a reference
/// so callers that need an owned value can `.cloned()` when the concrete
/// stage type is `Clone`.
pub fn advance_stage<'a, S: Stage>(current_id: Uuid, stages: &'a [S]) -> Option<&'a S> {
    let current = stages.iter().find(|s| s.id() == current_id)?;
    let current_order = current.order();
    stages
        .iter()
        .filter(|s| s.order() > current_order)
        .min_by_key(|s| s.order())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_line_subtotal_with_discount_and_tax() {
        // 100 units @ 10 each = 1000, 10% discount = 900, 20% tax = 1080
        let subtotal = line_subtotal(100.0, 10.0, 10.0, &[20.0]);
        assert!((subtotal - 1080.0).abs() < 0.01);
    }

    #[test]
    fn test_line_subtotal_no_discount() {
        // 50 units @ 5 each = 250, no discount, 10% tax = 275
        let subtotal = line_subtotal(50.0, 5.0, 0.0, &[10.0]);
        assert!((subtotal - 275.0).abs() < 0.01);
    }

    #[test]
    fn test_line_subtotal_no_tax() {
        // 20 units @ 15 each = 300, 20% discount = 240, no tax = 240
        let subtotal = line_subtotal(20.0, 15.0, 20.0, &[]);
        assert!((subtotal - 240.0).abs() < 0.01);
    }

    #[test]
    fn test_doc_total() {
        let totals = vec![100.0, 200.5, 50.0];
        assert!((doc_total(&totals) - 350.5).abs() < 0.01);
    }

    #[test]
    fn test_on_hand_by_location() {
        let loc_a = Uuid::new_v4();
        let loc_b = Uuid::new_v4();
        let loc_c = Uuid::new_v4();
        let prod = Uuid::new_v4();

        let moves = vec![
            StockMove {
                id: Uuid::new_v4(),
                product_id: prod,
                qty: 100.0,
                from_location_id: loc_a,
                to_location_id: loc_b,
                state: StockMoveState::Done,
                lot_id: None,
                created_at: chrono::Utc::now(),
                done_at: Some(chrono::Utc::now()),
            },
            StockMove {
                id: Uuid::new_v4(),
                product_id: prod,
                qty: 30.0,
                from_location_id: loc_b,
                to_location_id: loc_c,
                state: StockMoveState::Done,
                lot_id: None,
                created_at: chrono::Utc::now(),
                done_at: Some(chrono::Utc::now()),
            },
        ];

        let on_hand = on_hand_by_location(&moves, prod);
        assert_eq!(on_hand.get(&loc_a), Some(&-100.0));
        assert_eq!(on_hand.get(&loc_b), Some(&70.0));
        assert_eq!(on_hand.get(&loc_c), Some(&30.0));
    }

    #[test]
    fn test_on_hand_filters_cancelled_moves() {
        let loc_a = Uuid::new_v4();
        let loc_b = Uuid::new_v4();
        let prod = Uuid::new_v4();

        let moves = vec![
            StockMove {
                id: Uuid::new_v4(),
                product_id: prod,
                qty: 50.0,
                from_location_id: loc_a,
                to_location_id: loc_b,
                state: StockMoveState::Done,
                lot_id: None,
                created_at: chrono::Utc::now(),
                done_at: Some(chrono::Utc::now()),
            },
            StockMove {
                id: Uuid::new_v4(),
                product_id: prod,
                qty: 25.0,
                from_location_id: loc_a,
                to_location_id: loc_b,
                state: StockMoveState::Cancelled,
                lot_id: None,
                created_at: chrono::Utc::now(),
                done_at: None,
            },
        ];

        let on_hand = on_hand_by_location(&moves, prod);
        // Only the done move counts, so loc_b should have 50
        assert_eq!(on_hand.get(&loc_b), Some(&50.0));
    }

    #[test]
    fn test_explode_bom_simple() {
        let prod_a = Uuid::new_v4();
        let prod_b = Uuid::new_v4();

        let bom_a = Bom {
            id: Uuid::new_v4(),
            product_id: prod_a,
            components: vec![BomComponent {
                product_id: prod_b,
                qty: 3.0,
            }],
            quantity: 1.0,
            routing_id: None,
            created_at: chrono::Utc::now(),
        };

        let boms = std::iter::once((bom_a.id, bom_a.clone()))
            .collect::<HashMap<_, _>>();

        let explosion = explode_bom(&bom_a, &boms, 5.0);
        assert_eq!(explosion.len(), 1);
        assert_eq!(explosion[0].0, prod_b);
        assert!((explosion[0].1 - 15.0).abs() < 0.01);
    }

    #[test]
    fn test_explode_bom_multilevel() {
        let prod_a = Uuid::new_v4();
        let prod_b = Uuid::new_v4();
        let prod_c = Uuid::new_v4();

        let bom_b = Bom {
            id: Uuid::new_v4(),
            product_id: prod_b,
            components: vec![BomComponent {
                product_id: prod_c,
                qty: 2.0,
            }],
            quantity: 1.0,
            routing_id: None,
            created_at: chrono::Utc::now(),
        };

        let bom_a = Bom {
            id: Uuid::new_v4(),
            product_id: prod_a,
            components: vec![BomComponent {
                product_id: prod_b,
                qty: 4.0,
            }],
            quantity: 1.0,
            routing_id: None,
            created_at: chrono::Utc::now(),
        };

        let mut boms = HashMap::new();
        boms.insert(bom_a.id, bom_a.clone());
        boms.insert(bom_b.id, bom_b);

        let explosion = explode_bom(&bom_a, &boms, 2.0);
        // 2 units of A needs 8 units of B, which needs 16 units of C
        assert_eq!(explosion.len(), 1);
        assert_eq!(explosion[0].0, prod_c);
        assert!((explosion[0].1 - 16.0).abs() < 0.01);
    }
}
