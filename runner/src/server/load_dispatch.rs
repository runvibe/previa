use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};

use tokio::sync::Notify;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DispatchTick {
    pub elapsed_ms: u64,
    pub target_rps: f64,
    pub scheduled_starts: usize,
    pub scheduled_total: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DispatchTickReport {
    pub scheduled_starts: usize,
    pub actual_starts: usize,
    pub missed_starts: usize,
}

#[derive(Debug)]
pub struct DispatchClock {
    tick_ms: u64,
    fractional_carry: f64,
    scheduled_total: usize,
}

impl DispatchClock {
    pub fn new(tick_ms: u64) -> Self {
        Self {
            tick_ms,
            fractional_carry: 0.0,
            scheduled_total: 0,
        }
    }

    pub fn plan_tick(&mut self, elapsed_ms: u64, target_rps: f64) -> DispatchTick {
        let raw_slots = target_rps.max(0.0) * self.tick_ms as f64 / 1000.0 + self.fractional_carry;
        let scheduled_starts = raw_slots.floor() as usize;
        self.fractional_carry = raw_slots - scheduled_starts as f64;
        self.scheduled_total = self.scheduled_total.saturating_add(scheduled_starts);

        DispatchTick {
            elapsed_ms,
            target_rps,
            scheduled_starts,
            scheduled_total: self.scheduled_total,
        }
    }
}

#[derive(Debug)]
pub struct DispatchRuntimeState {
    generation: AtomicU64,
    slots: AtomicUsize,
    scheduled_in_tick: AtomicUsize,
    actual_in_tick: AtomicUsize,
    scheduled_total: AtomicUsize,
    waiters: AtomicUsize,
    closed: AtomicBool,
    notify: Notify,
}

impl DispatchRuntimeState {
    pub fn new() -> Self {
        Self {
            generation: AtomicU64::new(0),
            slots: AtomicUsize::new(0),
            scheduled_in_tick: AtomicUsize::new(0),
            actual_in_tick: AtomicUsize::new(0),
            scheduled_total: AtomicUsize::new(0),
            waiters: AtomicUsize::new(0),
            closed: AtomicBool::new(false),
            notify: Notify::new(),
        }
    }

    pub fn open_tick(&self, tick: DispatchTick) {
        if self.closed.load(Ordering::SeqCst) {
            return;
        }
        self.slots.store(tick.scheduled_starts, Ordering::SeqCst);
        self.scheduled_in_tick
            .store(tick.scheduled_starts, Ordering::SeqCst);
        self.actual_in_tick.store(0, Ordering::SeqCst);
        self.scheduled_total
            .store(tick.scheduled_total, Ordering::SeqCst);
        self.generation.fetch_add(1, Ordering::SeqCst);
        self.notify.notify_waiters();
    }

    pub fn close(&self) {
        self.closed.store(true, Ordering::SeqCst);
        self.slots.store(0, Ordering::SeqCst);
        self.notify.notify_waiters();
    }

    pub fn finish_tick(&self) -> DispatchTickReport {
        self.slots.store(0, Ordering::SeqCst);
        let scheduled_starts = self.scheduled_in_tick.swap(0, Ordering::SeqCst);
        let actual_starts = self.actual_in_tick.swap(0, Ordering::SeqCst);
        DispatchTickReport {
            scheduled_starts,
            actual_starts,
            missed_starts: scheduled_starts.saturating_sub(actual_starts),
        }
    }

    pub fn waiting_ready_requests(&self) -> usize {
        self.waiters.load(Ordering::SeqCst)
    }

    pub fn scheduled_total(&self) -> usize {
        self.scheduled_total.load(Ordering::SeqCst)
    }

    pub async fn acquire(&self, should_cancel: impl Fn() -> bool) -> bool {
        self.waiters.fetch_add(1, Ordering::SeqCst);
        let result = 'acquire: loop {
            if should_cancel() {
                break false;
            }
            if self.closed.load(Ordering::SeqCst) {
                break false;
            }

            let mut current = self.slots.load(Ordering::SeqCst);
            while current > 0 {
                match self.slots.compare_exchange(
                    current,
                    current - 1,
                    Ordering::SeqCst,
                    Ordering::SeqCst,
                ) {
                    Ok(_) => {
                        self.actual_in_tick.fetch_add(1, Ordering::SeqCst);
                        break 'acquire true;
                    }
                    Err(next) => current = next,
                }
            }

            self.notify.notified().await;
        };
        self.waiters.fetch_sub(1, Ordering::SeqCst);
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn schedules_exact_integer_slots_per_tick() {
        let mut clock = DispatchClock::new(100);

        let first = clock.plan_tick(0, 2400.0);
        assert_eq!(first.scheduled_starts, 240);
        assert_eq!(first.target_rps, 2400.0);

        let second = clock.plan_tick(100, 2400.0);
        assert_eq!(second.scheduled_starts, 240);
        assert_eq!(second.scheduled_total, 480);
    }

    #[test]
    fn carries_fractional_slots_without_backlog_debt() {
        let mut clock = DispatchClock::new(100);

        let first = clock.plan_tick(0, 15.0);
        assert_eq!(first.scheduled_starts, 1);

        let second = clock.plan_tick(100, 15.0);
        assert_eq!(second.scheduled_starts, 2);

        let third = clock.plan_tick(200, 15.0);
        assert_eq!(third.scheduled_starts, 1);
        assert_eq!(third.scheduled_total, 4);
    }

    #[test]
    fn does_not_repay_missed_slots_in_later_ticks() {
        let state = DispatchRuntimeState::new();
        state.open_tick(DispatchTick {
            elapsed_ms: 0,
            target_rps: 1000.0,
            scheduled_starts: 100,
            scheduled_total: 100,
        });

        assert_eq!(
            state.finish_tick(),
            DispatchTickReport {
                scheduled_starts: 100,
                actual_starts: 0,
                missed_starts: 100,
            }
        );

        state.open_tick(DispatchTick {
            elapsed_ms: 100,
            target_rps: 1000.0,
            scheduled_starts: 100,
            scheduled_total: 200,
        });

        assert_eq!(
            state.finish_tick(),
            DispatchTickReport {
                scheduled_starts: 100,
                actual_starts: 0,
                missed_starts: 100,
            }
        );
    }

    #[test]
    fn finish_tick_reports_each_tick_once() {
        let state = DispatchRuntimeState::new();
        state.open_tick(DispatchTick {
            elapsed_ms: 0,
            target_rps: 1000.0,
            scheduled_starts: 100,
            scheduled_total: 100,
        });

        assert_eq!(
            state.finish_tick(),
            DispatchTickReport {
                scheduled_starts: 100,
                actual_starts: 0,
                missed_starts: 100,
            }
        );
        assert_eq!(
            state.finish_tick(),
            DispatchTickReport {
                scheduled_starts: 0,
                actual_starts: 0,
                missed_starts: 0,
            }
        );
    }

    #[tokio::test]
    async fn closed_state_declines_without_consuming_slots() {
        let state = DispatchRuntimeState::new();
        state.open_tick(DispatchTick {
            elapsed_ms: 0,
            target_rps: 1000.0,
            scheduled_starts: 100,
            scheduled_total: 100,
        });

        state.close();

        assert!(!state.acquire(|| false).await);
        assert_eq!(
            state.finish_tick(),
            DispatchTickReport {
                scheduled_starts: 100,
                actual_starts: 0,
                missed_starts: 100,
            }
        );
    }
}
