use serde::{Deserialize, Serialize};

pub const QUEUE_PROTOCOL_VERSION: QueueProtocolVersion = QueueProtocolVersion(1);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct QueueProtocolVersion(pub i32);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionKind {
    E2e,
    Load,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionStatus {
    Queued,
    Running,
    CancelRequested,
    Completed,
    Failed,
    Cancelled,
}

impl ExecutionStatus {
    pub fn is_terminal(self) -> bool {
        matches!(self, Self::Completed | Self::Failed | Self::Cancelled)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JobStatus {
    Queued,
    Leased,
    Running,
    RetryWait,
    Completed,
    Failed,
    Cancelled,
    DeadLetter,
}

impl JobStatus {
    pub fn is_terminal(self) -> bool {
        matches!(
            self,
            Self::Completed | Self::Failed | Self::Cancelled | Self::DeadLetter
        )
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::{ExecutionKind, ExecutionStatus, JobStatus};

    #[test]
    fn queue_states_use_stable_snake_case_wire_values() {
        assert_eq!(
            serde_json::to_value(ExecutionKind::E2e).unwrap(),
            json!("e2e")
        );
        assert_eq!(
            serde_json::to_value(ExecutionStatus::CancelRequested).unwrap(),
            json!("cancel_requested")
        );
        assert_eq!(
            serde_json::to_value(JobStatus::DeadLetter).unwrap(),
            json!("dead_letter")
        );
    }

    #[test]
    fn terminal_states_are_explicit() {
        assert!(ExecutionStatus::Completed.is_terminal());
        assert!(ExecutionStatus::Failed.is_terminal());
        assert!(ExecutionStatus::Cancelled.is_terminal());
        assert!(!ExecutionStatus::Running.is_terminal());
        assert!(JobStatus::DeadLetter.is_terminal());
        assert!(!JobStatus::RetryWait.is_terminal());
    }
}
