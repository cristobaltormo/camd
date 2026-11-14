use std::collections::{HashMap, VecDeque};
use std::sync::Mutex;

use crate::clock;

pub const MAX_KEYS: usize = 4096;

struct Hits {
    keys: HashMap<String, VecDeque<f64>>,
    pruned_at: f64,
}

pub struct RateLimiter {
    pub limit: usize,
    pub window: f64,
    hits: Mutex<Hits>,
}

impl RateLimiter {
    pub fn new(limit: usize, window: f64) -> RateLimiter {
        RateLimiter {
            limit,
            window,
            hits: Mutex::new(Hits {
                keys: HashMap::new(),
                pruned_at: 0.0,
            }),
        }
    }

    pub fn allow(&self, key: &str) -> bool {
        self.allow_at(key, clock::now())
    }

    pub fn tracked(&self) -> usize {
        self.hits
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .keys
            .len()
    }

    pub fn allow_at(&self, key: &str, now: f64) -> bool {
        let mut hits = self.hits.lock().unwrap_or_else(|e| e.into_inner());
        let window = self.window;
        if now - hits.pruned_at > window || hits.keys.len() >= MAX_KEYS {
            hits.keys
                .retain(|_, list| list.back().is_none_or(|last| now - last <= window));
            hits.pruned_at = now;
            if hits.keys.len() >= MAX_KEYS {
                hits.keys.clear();
            }
        }
        let list = hits.keys.entry(key.to_string()).or_default();
        while list.front().is_some_and(|first| now - first > window) {
            list.pop_front();
        }
        if list.len() >= self.limit {
            return false;
        }
        list.push_back(now);
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ten_attempts_per_minute_per_key() {
        let limiter = RateLimiter::new(10, 60.0);
        let allowed = (0..14)
            .filter(|i| limiter.allow_at("1.2.3.4", 1000.0 + *i as f64))
            .count();
        assert_eq!(allowed, 10);
        assert!(limiter.allow_at("5.6.7.8", 1014.0));
        assert!(limiter.allow_at("1.2.3.4", 1000.0 + 61.0));
    }

    #[test]
    fn spoofed_keys_stay_bounded_and_old_keys_are_forgotten() {
        let limiter = RateLimiter::new(10, 60.0);
        for i in 0..3 * MAX_KEYS {
            assert!(limiter.allow_at(&format!("10.0.{}.{}", i / 256, i % 256), 1000.0));
        }
        assert!(limiter.tracked() <= MAX_KEYS);
        limiter.allow_at("1.2.3.4", 1100.0);
        assert_eq!(limiter.tracked(), 1);
    }
}
