//! Erasure-coding plan — Reed-Solomon (data, parity) shard layout.
//!
//! Mirrors MinIO's `EcConfig` with `data_blocks` + `parity_blocks` per
//! storage class. We don't include the actual encoder here (RS encode/decode
//! is left to a backend integration) but do model the shard layout, capacity
//! math, and the recovery threshold check that MinIO's healing path uses.

#[derive(Debug, thiserror::Error, PartialEq)]
pub enum ErasureError {
    #[error("invalid block count: {0}")]
    InvalidBlockCount(usize),
    #[error("not enough shards: have {have}, need {need}")]
    InsufficientShards { have: usize, need: usize },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ErasureConfig {
    pub data_blocks: usize,
    pub parity_blocks: usize,
}

impl ErasureConfig {
    pub fn new(data: usize, parity: usize) -> Result<Self, ErasureError> {
        if data == 0 || parity == 0 {
            return Err(ErasureError::InvalidBlockCount(data + parity));
        }
        if data + parity > 16 {
            return Err(ErasureError::InvalidBlockCount(data + parity));
        }
        Ok(Self {
            data_blocks: data,
            parity_blocks: parity,
        })
    }

    pub fn total_shards(&self) -> usize {
        self.data_blocks + self.parity_blocks
    }

    /// Minimum number of shards required to reconstruct an object.
    pub fn min_shards_for_read(&self) -> usize {
        self.data_blocks
    }

    /// Number of shards that may be lost while still reading.
    pub fn redundancy(&self) -> usize {
        self.parity_blocks
    }

    /// Storage overhead ratio: total / data.
    pub fn overhead_ratio(&self) -> f64 {
        self.total_shards() as f64 / self.data_blocks as f64
    }

    /// Authorize a read given the number of shards currently online.
    pub fn authorize_read(&self, online_shards: usize) -> Result<(), ErasureError> {
        if online_shards < self.min_shards_for_read() {
            return Err(ErasureError::InsufficientShards {
                have: online_shards,
                need: self.min_shards_for_read(),
            });
        }
        Ok(())
    }

    /// Number of shards that need rebuilding given how many are online.
    pub fn rebuild_count(&self, online_shards: usize) -> usize {
        if online_shards >= self.total_shards() {
            return 0;
        }
        self.total_shards() - online_shards
    }
}

/// Standard MinIO defaults.
impl ErasureConfig {
    pub fn ec_4() -> Self {
        Self::new(8, 4).unwrap()
    }
    pub fn ec_2() -> Self {
        Self::new(10, 2).unwrap()
    }
    pub fn ec_1() -> Self {
        Self::new(11, 1).unwrap()
    }
    pub fn ec_8() -> Self {
        Self::new(8, 8).unwrap()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_valid_config() {
        let c = ErasureConfig::new(8, 4).unwrap();
        assert_eq!(c.total_shards(), 12);
    }

    #[test]
    fn new_zero_data_errors() {
        assert!(ErasureConfig::new(0, 4).is_err());
    }

    #[test]
    fn new_zero_parity_errors() {
        assert!(ErasureConfig::new(8, 0).is_err());
    }

    #[test]
    fn new_total_above_16_errors() {
        assert!(ErasureConfig::new(10, 10).is_err());
    }

    #[test]
    fn min_shards_equals_data_blocks() {
        let c = ErasureConfig::ec_4();
        assert_eq!(c.min_shards_for_read(), 8);
    }

    #[test]
    fn redundancy_equals_parity_blocks() {
        let c = ErasureConfig::ec_4();
        assert_eq!(c.redundancy(), 4);
    }

    #[test]
    fn overhead_ratio_correct() {
        let c = ErasureConfig::ec_4();
        // 12/8 = 1.5
        assert!((c.overhead_ratio() - 1.5).abs() < 1e-9);
    }

    #[test]
    fn authorize_read_ok_with_enough_shards() {
        let c = ErasureConfig::ec_4();
        assert!(c.authorize_read(8).is_ok());
        assert!(c.authorize_read(12).is_ok());
    }

    #[test]
    fn authorize_read_fails_below_threshold() {
        let c = ErasureConfig::ec_4();
        let err = c.authorize_read(7).unwrap_err();
        assert!(matches!(err, ErasureError::InsufficientShards { .. }));
    }

    #[test]
    fn rebuild_count_when_full() {
        let c = ErasureConfig::ec_4();
        assert_eq!(c.rebuild_count(12), 0);
    }

    #[test]
    fn rebuild_count_when_partial() {
        let c = ErasureConfig::ec_4();
        assert_eq!(c.rebuild_count(10), 2);
    }

    #[test]
    fn ec_1_through_ec_8_presets() {
        assert_eq!(ErasureConfig::ec_1().redundancy(), 1);
        assert_eq!(ErasureConfig::ec_2().redundancy(), 2);
        assert_eq!(ErasureConfig::ec_4().redundancy(), 4);
        assert_eq!(ErasureConfig::ec_8().redundancy(), 8);
    }

    #[test]
    fn ec_1_total_shards() {
        let c = ErasureConfig::ec_1();
        assert_eq!(c.total_shards(), 12);
    }

    #[test]
    fn rebuild_count_when_zero_online() {
        let c = ErasureConfig::ec_4();
        assert_eq!(c.rebuild_count(0), 12);
    }
}
