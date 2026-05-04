use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use crate::server::load_dispatch::DispatchClock;
use crate::server::models::LoadProfile;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct WaveDispatchSlot {
    pub elapsed_ms: u64,
    pub planned_starts: usize,
    pub target_rps_limit: f64,
    pub scheduled_total: usize,
    pub scheduler_lag_ms: u64,
    pub missed_due_to_scheduler_lag: usize,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum WaveSchedulerMetric {
    DispatchScheduled { count: usize },
    SchedulerLag { lag_ms: u64, missed_starts: usize },
    SlotBackpressure { dropped_starts: usize },
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
        Ok(()) => true,
        Err(mpsc::error::TrySendError::Full(slot)) => {
            let _ = metric_tx.send(WaveSchedulerMetric::SlotBackpressure {
                dropped_starts: slot.planned_starts,
            });
            false
        }
        Err(mpsc::error::TrySendError::Closed(_)) => false,
    }
}

pub async fn run_wave_scheduler(
    load: LoadProfile,
    tick_ms: u64,
    slot_tx: mpsc::Sender<WaveDispatchSlot>,
    metric_tx: mpsc::UnboundedSender<WaveSchedulerMetric>,
    token: CancellationToken,
) {
    let started = std::time::Instant::now();
    let end_ms = crate::server::load_wave::timeline_end_ms(&load);
    let mut clock = DispatchClock::new(tick_ms);

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

        let _ = try_send_slot_or_metric(
            &slot_tx,
            &metric_tx,
            WaveDispatchSlot {
                elapsed_ms: tick.elapsed_ms,
                planned_starts: tick.scheduled_starts,
                target_rps_limit,
                scheduled_total: tick.scheduled_total,
                scheduler_lag_ms: tick.scheduler_lag_ms,
                missed_due_to_scheduler_lag: tick.missed_due_to_scheduler_lag,
            },
        );

        tokio::time::sleep(tokio::time::Duration::from_millis(tick_ms)).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_slot_from_clock_tick_uses_tick_window_only() {
        let mut clock = DispatchClock::new(100);
        let tick = clock.plan_tick(500, 100.0);
        let slot = WaveDispatchSlot {
            elapsed_ms: tick.elapsed_ms,
            planned_starts: tick.scheduled_starts,
            target_rps_limit: tick.target_rps,
            scheduled_total: tick.scheduled_total,
            scheduler_lag_ms: tick.scheduler_lag_ms,
            missed_due_to_scheduler_lag: tick.missed_due_to_scheduler_lag,
        };

        assert_eq!(slot.planned_starts, 10);
        assert_eq!(slot.elapsed_ms, 500);
        assert_eq!(slot.target_rps_limit, 100.0);
    }

    #[tokio::test]
    async fn scheduler_emits_metric_when_slot_channel_is_full() {
        let (slot_tx, mut slot_rx) = mpsc::channel(1);
        let (metric_tx, mut metric_rx) = mpsc::unbounded_channel();

        slot_tx
            .send(WaveDispatchSlot {
                elapsed_ms: 0,
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
            Some(WaveSchedulerMetric::SlotBackpressure { dropped_starts: 7 })
        ));

        assert!(slot_rx.recv().await.is_some());
    }
}
