use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum SamplingStrategy {
    Const {
        sample: bool,
    },
    Probabilistic {
        sampling_rate: f64,
    },
    RateLimiting {
        max_traces_per_second: f64,
    },
    Remote {
        endpoint: String,
        default_strategy: Box<SamplingStrategy>,
    },
}

pub struct Sampler {
    strategy: SamplingStrategy,
    counter: Arc<AtomicU64>,
    last_reset: Arc<std::sync::Mutex<std::time::Instant>>,
}

impl Sampler {
    pub fn new(strategy: SamplingStrategy) -> Self {
        Sampler {
            strategy,
            counter: Arc::new(AtomicU64::new(0)),
            last_reset: Arc::new(std::sync::Mutex::new(std::time::Instant::now())),
        }
    }

    pub fn should_sample(&self, trace_id: &str, operation: &str) -> bool {
        match &self.strategy {
            SamplingStrategy::Const { sample } => *sample,
            SamplingStrategy::Probabilistic { sampling_rate } => {
                let hash = simple_hash(trace_id);
                (hash as f64 / u64::MAX as f64) < *sampling_rate
            }
            SamplingStrategy::RateLimiting {
                max_traces_per_second,
            } => {
                let now = std::time::Instant::now();
                let mut last = self.last_reset.lock().unwrap();
                if now.duration_since(*last).as_secs() >= 1 {
                    self.counter.store(0, Ordering::Relaxed);
                    *last = now;
                }
                let count = self.counter.fetch_add(1, Ordering::Relaxed);
                (count as f64) < *max_traces_per_second
            }
            SamplingStrategy::Remote {
                default_strategy, ..
            } => {
                let inner = Sampler::new(*default_strategy.clone());
                inner.should_sample(trace_id, operation)
            }
        }
    }

    pub fn sampling_rate(&self) -> f64 {
        match &self.strategy {
            SamplingStrategy::Const { sample } => {
                if *sample {
                    1.0
                } else {
                    0.0
                }
            }
            SamplingStrategy::Probabilistic { sampling_rate } => *sampling_rate,
            SamplingStrategy::RateLimiting {
                max_traces_per_second,
            } => *max_traces_per_second,
            SamplingStrategy::Remote {
                default_strategy, ..
            } => Sampler::new(*default_strategy.clone()).sampling_rate(),
        }
    }
}

fn simple_hash(s: &str) -> u64 {
    let mut h: u64 = 14695981039346656037;
    for b in s.bytes() {
        h = h.wrapping_mul(1099511628211);
        h ^= b as u64;
    }
    h
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sampling_const_always() {
        let sampler = Sampler::new(SamplingStrategy::Const { sample: true });
        for i in 0..20 {
            assert!(sampler.should_sample(&format!("trace-{i}"), "op"), "should always sample");
        }
        assert_eq!(sampler.sampling_rate(), 1.0);
    }

    #[test]
    fn sampling_const_never() {
        let sampler = Sampler::new(SamplingStrategy::Const { sample: false });
        for i in 0..20 {
            assert!(
                !sampler.should_sample(&format!("trace-{i}"), "op"),
                "should never sample"
            );
        }
        assert_eq!(sampler.sampling_rate(), 0.0);
    }

    #[test]
    fn sampling_probabilistic() {
        // 0.0 never samples
        let never = Sampler::new(SamplingStrategy::Probabilistic { sampling_rate: 0.0 });
        for i in 0..50 {
            assert!(
                !never.should_sample(&format!("trace-{i}"), "op"),
                "rate 0.0 should never sample"
            );
        }

        // 1.0 always samples
        let always = Sampler::new(SamplingStrategy::Probabilistic { sampling_rate: 1.0 });
        for i in 0..50 {
            assert!(
                always.should_sample(&format!("trace-{i}"), "op"),
                "rate 1.0 should always sample"
            );
        }

        // Check rate is stored
        assert_eq!(always.sampling_rate(), 1.0);
    }

    #[test]
    fn sampling_rate_limiting() {
        let limit = 5.0;
        let sampler = Sampler::new(SamplingStrategy::RateLimiting {
            max_traces_per_second: limit,
        });
        let mut sampled = 0;
        for i in 0..10 {
            if sampler.should_sample(&format!("t{i}"), "op") {
                sampled += 1;
            }
        }
        // Within the same second, at most `limit` (5) should be sampled
        assert_eq!(sampled, limit as i32);
    }
}
