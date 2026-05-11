#[cfg(test)]
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};

#[cfg(test)]
use tokio::sync::Notify;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DispatchTick {
    pub elapsed_ms: u64,
    pub target_rps: f64,
    pub scheduled_starts: usize,
    pub scheduled_total: usize,
    pub scheduler_lag_ms: u64,
    pub missed_due_to_scheduler_lag: usize,
}

#[cfg(test)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DispatchTickReport {
    pub scheduled_starts: usize,
    pub actual_starts: usize,
    pub missed_starts: usize,
}

#[derive(Debug)]
pub struct DispatchClock {
    tick_ms: u64,
    cursor_elapsed_ms: u64,
    fractional_carry: f64,
    scheduled_total: usize,
}

impl DispatchClock {
    pub fn new(tick_ms: u64) -> Self {
        Self {
            tick_ms,
            cursor_elapsed_ms: 0,
            fractional_carry: 0.0,
            scheduled_total: 0,
        }
    }

    pub fn plan_tick(&mut self, elapsed_ms: u64, target_rps: f64) -> DispatchTick {
        let scheduler_lag_ms = elapsed_ms.saturating_sub(self.cursor_elapsed_ms);
        let missed_raw = target_rps.max(0.0) * scheduler_lag_ms as f64 / 1000.0;
        let missed_due_to_scheduler_lag = missed_raw.floor() as usize;
        let raw_slots = target_rps.max(0.0) * self.tick_ms as f64 / 1000.0 + self.fractional_carry;
        let scheduled_starts = raw_slots.floor() as usize;
        self.fractional_carry = raw_slots - scheduled_starts as f64;
        self.scheduled_total = self.scheduled_total.saturating_add(scheduled_starts);
        self.cursor_elapsed_ms = elapsed_ms.saturating_add(self.tick_ms);

        DispatchTick {
            elapsed_ms,
            target_rps,
            scheduled_starts,
            scheduled_total: self.scheduled_total,
            scheduler_lag_ms,
            missed_due_to_scheduler_lag,
        }
    }
}

#[cfg(test)]
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

#[cfg(test)]
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
    fn delayed_tick_records_lag_without_repaying_missed_wall_time() {
        let mut clock = DispatchClock::new(100);

        let first = clock.plan_tick(0, 1000.0);
        assert_eq!(first.scheduled_starts, 100);
        assert_eq!(first.scheduler_lag_ms, 0);
        assert_eq!(first.missed_due_to_scheduler_lag, 0);

        let delayed = clock.plan_tick(500, 1000.0);
        assert_eq!(delayed.scheduled_starts, 100);
        assert_eq!(delayed.scheduler_lag_ms, 400);
        assert_eq!(delayed.missed_due_to_scheduler_lag, 400);
        assert_eq!(delayed.scheduled_total, 200);
    }

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
    fn delayed_tick_keeps_next_window_size_after_reporting_lag() {
        let mut clock = DispatchClock::new(100);

        let first = clock.plan_tick(0, 1000.0);
        assert_eq!(first.scheduled_starts, 100);

        let delayed = clock.plan_tick(500, 1000.0);
        assert_eq!(delayed.scheduled_starts, 100);
        assert_eq!(delayed.scheduler_lag_ms, 400);
        assert_eq!(delayed.missed_due_to_scheduler_lag, 400);
        assert_eq!(delayed.scheduled_total, 200);
    }

    #[test]
    fn dispatch_clock_is_independent_from_failures_by_design() {
        let mut clock = DispatchClock::new(100);

        let a = clock.plan_tick(0, 1000.0);
        let b = clock.plan_tick(100, 1000.0);
        let c = clock.plan_tick(200, 1000.0);

        assert_eq!(a.scheduled_starts, 100);
        assert_eq!(b.scheduled_starts, 100);
        assert_eq!(c.scheduled_starts, 100);
        assert_eq!(c.scheduled_total, 300);
    }

    #[test]
    fn does_not_repay_missed_slots_in_later_ticks() {
        let state = DispatchRuntimeState::new();
        state.open_tick(DispatchTick {
            elapsed_ms: 0,
            target_rps: 1000.0,
            scheduled_starts: 100,
            scheduled_total: 100,
            scheduler_lag_ms: 0,
            missed_due_to_scheduler_lag: 0,
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
            scheduler_lag_ms: 0,
            missed_due_to_scheduler_lag: 0,
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
            scheduler_lag_ms: 0,
            missed_due_to_scheduler_lag: 0,
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
            scheduler_lag_ms: 0,
            missed_due_to_scheduler_lag: 0,
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
