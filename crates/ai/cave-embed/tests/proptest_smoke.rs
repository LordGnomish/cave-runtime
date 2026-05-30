// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//
// Property-based invariants for the embedding pipeline (workspace convention).

use cave_embed::backend::{EmbeddingBackend, HashEmbedder};
use cave_embed::pooling::{l2_normalize, Pooling};
use cave_embed::quant::{f16_to_f32, f32_to_f16};
use cave_embed::registry::ModelCard;
use proptest::prelude::*;

proptest! {
    // L2 normalization always yields unit length (or an all-zero vector).
    #[test]
    fn l2_normalize_is_unit_or_zero(xs in prop::collection::vec(-1e3f32..1e3, 1..32)) {
        let n = l2_normalize(&xs);
        let mag: f32 = n.iter().map(|x| x * x).sum::<f32>().sqrt();
        prop_assert!(mag == 0.0 || (mag - 1.0).abs() < 1e-3);
    }

    // The reference embedder is deterministic and dimension-faithful.
    #[test]
    fn embedder_dim_and_determinism(text in "[a-z ]{0,64}", dim in 8usize..256) {
        let be = HashEmbedder::new();
        let card = ModelCard::text("t", dim, 256, Pooling::Mean, true);
        let a = be.embed(&[text.clone()], &card).unwrap();
        let b = be.embed(&[text], &card).unwrap();
        prop_assert_eq!(&a, &b);
        prop_assert_eq!(a[0].len(), dim);
    }

    // fp16 round-trip never produces NaN for finite inputs and stays close.
    #[test]
    fn fp16_round_trip_finite(x in -1000.0f32..1000.0) {
        let back = f16_to_f32(f32_to_f16(x));
        prop_assert!(back.is_finite());
        let tol = (x.abs() * 1e-2).max(1.0);
        prop_assert!((back - x).abs() <= tol);
    }
}
