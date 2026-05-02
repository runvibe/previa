#[derive(Debug, Clone)]
pub struct FlowBucket {
    capacity: f64,
    tokens: f64,
    last_refill_ms: u64,
}

impl FlowBucket {
    pub fn new(initial_rps: f64, now_ms: u64) -> Self {
        let capacity = initial_rps.max(0.0);
        Self {
            capacity,
            tokens: capacity,
            last_refill_ms: now_ms,
        }
    }

    pub fn refill(&mut self, rps_limit: f64, now_ms: u64) {
        let next_capacity = rps_limit.max(0.0);
        let elapsed_ms = now_ms.saturating_sub(self.last_refill_ms) as f64;
        let earned = next_capacity * elapsed_ms / 1000.0;
        self.capacity = next_capacity;
        self.tokens = (self.tokens + earned).min(self.capacity);
        self.last_refill_ms = now_ms;
    }

    pub fn try_acquire(&mut self) -> bool {
        if self.tokens >= 1.0 {
            self.tokens -= 1.0;
            true
        } else {
            false
        }
    }

    #[cfg(test)]
    pub fn available_tokens(&self) -> f64 {
        self.tokens
    }
}

#[cfg(test)]
mod tests {
    use super::FlowBucket;

    #[test]
    fn admits_when_tokens_are_available() {
        let mut bucket = FlowBucket::new(10.0, 0);
        bucket.refill(10.0, 0);

        assert!(bucket.try_acquire());
        assert_eq!(bucket.available_tokens().floor(), 9.0);
    }

    #[test]
    fn blocks_when_empty_until_refilled() {
        let mut bucket = FlowBucket::new(1.0, 0);
        bucket.refill(1.0, 0);

        assert!(bucket.try_acquire());
        assert!(!bucket.try_acquire());

        bucket.refill(1.0, 1000);
        assert!(bucket.try_acquire());
    }

    #[test]
    fn updates_capacity_when_limit_changes() {
        let mut bucket = FlowBucket::new(100.0, 0);
        bucket.refill(100.0, 1000);
        assert!(bucket.available_tokens() <= 100.0);

        bucket.refill(10.0, 2000);
        assert!(bucket.available_tokens() <= 10.0);
    }
}
