use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use crate::server::load_dispatch::DispatchClock;
use crate::server::models::LoadProfile;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct WaveDispatchSlot {
    pub elapsed_ms: u64,
    pub expires_at_elapsed_ms: u64,
    pub planned_starts: usize,
    pub target_rps_limit: f64,
    pub scheduled_total: usize,
    pub scheduler_lag_ms: u64,
    pub missed_due_to_scheduler_lag: usize,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum WaveSchedulerMetric {
    DispatchScheduled {
        elapsed_ms: u64,
        count: usize,
    },
    SlotEnqueued {
        elapsed_ms: u64,
        count: usize,
    },
    SchedulerLag {
        elapsed_ms: u64,
        lag_ms: u64,
        missed_starts: usize,
    },
    SlotBackpressure {
        elapsed_ms: u64,
        dropped_starts: usize,
    },
}

pub fn build_dispatch_slot(
    tick: crate::server::load_dispatch::DispatchTick,
    tick_ms: u64,
) -> WaveDispatchSlot {
    WaveDispatchSlot {
        elapsed_ms: tick.elapsed_ms,
        expires_at_elapsed_ms: tick.elapsed_ms.saturating_add(tick_ms),
        planned_starts: tick.scheduled_starts,
        target_rps_limit: tick.target_rps,
        scheduled_total: tick.scheduled_total,
        scheduler_lag_ms: tick.scheduler_lag_ms,
        missed_due_to_scheduler_lag: tick.missed_due_to_scheduler_lag,
    }
}

pub fn slot_is_expired(slot: &WaveDispatchSlot, actual_elapsed_ms: u64) -> bool {
    actual_elapsed_ms > slot.expires_at_elapsed_ms
}

pub fn try_send_slot_or_metric(
    slot_tx: &mpsc::Sender<WaveDispatchSlot>,
    metric_tx: &mpsc::UnboundedSender<WaveSchedulerMetric>,
    slot: WaveDispatchSlot,
) -> bool {
    if slot.planned_starts == 0 {
        return true;
    }

    match slot_tx.try_send(slot) {
        Ok(()) => {
            let _ = metric_tx.send(WaveSchedulerMetric::SlotEnqueued {
                elapsed_ms: slot.elapsed_ms,
                count: slot.planned_starts,
            });
            true
        }
        Err(mpsc::error::TrySendError::Full(slot)) => {
            let _ = metric_tx.send(WaveSchedulerMetric::SlotBackpressure {
                elapsed_ms: slot.elapsed_ms,
                dropped_starts: slot.planned_starts,
            });
            false
        }
        Err(mpsc::error::TrySendError::Closed(_)) => false,
    }
}

pub fn spawn_wave_scheduler_thread(
    load: LoadProfile,
    tick_ms: u64,
    slot_tx: mpsc::Sender<WaveDispatchSlot>,
    metric_tx: mpsc::UnboundedSender<WaveSchedulerMetric>,
    token: CancellationToken,
) -> std::thread::JoinHandle<()> {
    std::thread::Builder::new()
        .name("previa-wave-clock".to_owned())
        .spawn(move || run_wave_scheduler_loop(load, tick_ms, slot_tx, metric_tx, token))
        .expect("failed to spawn previa wave scheduler thread")
}

pub fn run_wave_scheduler_loop(
    load: LoadProfile,
    tick_ms: u64,
    slot_tx: mpsc::Sender<WaveDispatchSlot>,
    metric_tx: mpsc::UnboundedSender<WaveSchedulerMetric>,
    token: CancellationToken,
) {
    let started = std::time::Instant::now();
    let end_ms = crate::server::load_wave::timeline_end_ms(&load);
    let mut clock = DispatchClock::new(tick_ms);
    let mut next_wake = started;

    loop {
        if token.is_cancelled() {
            break;
        }

        let elapsed_ms = started.elapsed().as_millis() as u64;
        if elapsed_ms >= end_ms {
            break;
        }

        let target_rps_limit = crate::server::load_wave::local_rps_limit(&load, elapsed_ms);
        let tick = clock.plan_tick(elapsed_ms, target_rps_limit);
        let _ = metric_tx.send(WaveSchedulerMetric::DispatchScheduled {
            elapsed_ms: tick.elapsed_ms,
            count: tick.scheduled_starts,
        });
        if tick.scheduler_lag_ms > 0 || tick.missed_due_to_scheduler_lag > 0 {
            let _ = metric_tx.send(WaveSchedulerMetric::SchedulerLag {
                elapsed_ms: tick.elapsed_ms,
                lag_ms: tick.scheduler_lag_ms,
                missed_starts: tick.missed_due_to_scheduler_lag,
            });
        }
        let _ = try_send_slot_or_metric(&slot_tx, &metric_tx, build_dispatch_slot(tick, tick_ms));

        next_wake += std::time::Duration::from_millis(tick_ms);
        let now = std::time::Instant::now();
        if next_wake > now {
            std::thread::sleep(next_wake - now);
        } else {
            next_wake = now;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_slot_from_clock_tick_uses_tick_window_only() {
        let mut clock = DispatchClock::new(100);
        let tick = clock.plan_tick(500, 100.0);
        let slot = build_dispatch_slot(tick, 100);

        assert_eq!(slot.planned_starts, 10);
        assert_eq!(slot.elapsed_ms, 500);
        assert_eq!(slot.expires_at_elapsed_ms, 600);
        assert_eq!(slot.target_rps_limit, 100.0);
    }

    #[test]
    fn dispatch_slot_is_fresh_until_expiration_elapsed_ms() {
        let slot = WaveDispatchSlot {
            elapsed_ms: 500,
            expires_at_elapsed_ms: 600,
            planned_starts: 10,
            target_rps_limit: 100.0,
            scheduled_total: 20,
            scheduler_lag_ms: 0,
            missed_due_to_scheduler_lag: 0,
        };

        assert!(!slot_is_expired(&slot, 599));
        assert!(!slot_is_expired(&slot, 600));
        assert!(slot_is_expired(&slot, 601));
    }

    #[test]
    fn build_slot_from_clock_tick_sets_expiration_to_next_tick_boundary() {
        let mut clock = DispatchClock::new(100);
        let tick = clock.plan_tick(500, 100.0);
        let slot = build_dispatch_slot(tick, 100);

        assert_eq!(slot.elapsed_ms, 500);
        assert_eq!(slot.expires_at_elapsed_ms, 600);
        assert_eq!(slot.planned_starts, 10);
        assert_eq!(slot.target_rps_limit, 100.0);
    }

    fn short_flat_load() -> LoadProfile {
        LoadProfile {
            points: vec![
                crate::server::models::LoadPoint {
                    at_ms: 0,
                    intensity: 50.0,
                },
                crate::server::models::LoadPoint {
                    at_ms: 250,
                    intensity: 50.0,
                },
            ],
            interpolation: crate::server::models::LoadInterpolation::Linear,
            runner_max_rps: 1000.0,
            grace_period_ms: 0,
        }
    }

    #[tokio::test]
    async fn scheduler_thread_emits_slots_without_tokio_spawn() {
        let (slot_tx, mut slot_rx) = mpsc::channel(8);
        let (metric_tx, _metric_rx) = mpsc::unbounded_channel();
        let token = CancellationToken::new();

        let handle = spawn_wave_scheduler_thread(short_flat_load(), 100, slot_tx, metric_tx, token);

        let first = tokio::time::timeout(std::time::Duration::from_millis(300), slot_rx.recv())
            .await
            .expect("scheduler thread should emit a slot")
            .expect("slot channel should stay open while scheduler runs");

        assert!(first.planned_starts > 0);
        assert_eq!(first.expires_at_elapsed_ms, first.elapsed_ms + 100);

        handle.join().expect("scheduler thread should exit cleanly");
    }

    #[tokio::test]
    async fn scheduler_thread_emits_dispatch_scheduled_metric() {
        let (slot_tx, mut slot_rx) = mpsc::channel(8);
        let (metric_tx, mut metric_rx) = mpsc::unbounded_channel();
        let token = CancellationToken::new();

        let handle = spawn_wave_scheduler_thread(short_flat_load(), 100, slot_tx, metric_tx, token);

        let metric = tokio::time::timeout(std::time::Duration::from_millis(300), metric_rx.recv())
            .await
            .expect("scheduler should emit a metric")
            .expect("metric channel should stay open");

        assert!(matches!(
            metric,
            WaveSchedulerMetric::DispatchScheduled { count, .. } if count > 0
        ));
        assert!(slot_rx.recv().await.is_some());

        handle.join().expect("scheduler thread should exit cleanly");
    }

    #[tokio::test]
    async fn scheduler_emits_metric_when_slot_channel_is_full() {
        let (slot_tx, mut slot_rx) = mpsc::channel(1);
        let (metric_tx, mut metric_rx) = mpsc::unbounded_channel();

        slot_tx
            .send(WaveDispatchSlot {
                elapsed_ms: 0,
                expires_at_elapsed_ms: 100,
                planned_starts: 1,
                target_rps_limit: 10.0,
                scheduled_total: 1,
                scheduler_lag_ms: 0,
                missed_due_to_scheduler_lag: 0,
            })
            .await
            .unwrap();

        let sent = try_send_slot_or_metric(
            &slot_tx,
            &metric_tx,
            WaveDispatchSlot {
                elapsed_ms: 100,
                expires_at_elapsed_ms: 200,
                planned_starts: 7,
                target_rps_limit: 70.0,
                scheduled_total: 8,
                scheduler_lag_ms: 0,
                missed_due_to_scheduler_lag: 0,
            },
        );

        assert!(!sent);
        assert!(matches!(
            metric_rx.recv().await,
            Some(WaveSchedulerMetric::SlotBackpressure {
                elapsed_ms: 100,
                dropped_starts: 7,
            })
        ));

        assert!(slot_rx.recv().await.is_some());
    }
}
