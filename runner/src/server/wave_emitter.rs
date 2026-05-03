#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StartLagClass {
    OnTime,
    RuntimeLagged,
}

pub fn classify_start_lag(
    tick_elapsed_ms: u64,
    actual_elapsed_ms: u64,
    tick_ms: u64,
) -> StartLagClass {
    if actual_elapsed_ms <= tick_elapsed_ms.saturating_add(tick_ms) {
        StartLagClass::OnTime
    } else {
        StartLagClass::RuntimeLagged
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_runtime_lag_when_send_start_happens_after_tick_window() {
        let tick_ms = 100;
        let lag = classify_start_lag(1_000, 1_135, tick_ms);
        assert_eq!(lag, StartLagClass::RuntimeLagged);
    }

    #[test]
    fn accepts_start_inside_tick_window() {
        let tick_ms = 100;
        let lag = classify_start_lag(1_000, 1_075, tick_ms);
        assert_eq!(lag, StartLagClass::OnTime);
    }
}
