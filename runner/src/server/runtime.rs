use sysinfo::{Pid, ProcessRefreshKind, ProcessesToUpdate, System};

use crate::server::models::RunnerInfoResponse;

#[derive(Debug)]
pub struct RuntimeSampler {
    pid: u32,
    process_pid: Pid,
    system: System,
}

impl RuntimeSampler {
    pub fn new() -> Self {
        let pid = std::process::id();
        let process_pid = Pid::from_u32(pid);
        let mut sampler = Self {
            pid,
            process_pid,
            system: System::new(),
        };
        sampler.refresh();
        sampler
    }

    pub fn snapshot(&mut self) -> Option<RunnerInfoResponse> {
        self.refresh();
        let process = self.system.process(self.process_pid)?;
        Some(RunnerInfoResponse {
            pid: self.pid,
            memory_bytes: process.memory(),
            virtual_memory_bytes: process.virtual_memory(),
            cpu_usage_percent: process.cpu_usage(),
            network_tx_bytes: 0,
            network_rx_bytes: 0,
            network_total_bytes: 0,
        })
    }

    fn refresh(&mut self) {
        self.system.refresh_processes_specifics(
            ProcessesToUpdate::Some(&[self.process_pid]),
            true,
            ProcessRefreshKind::nothing().with_memory().with_cpu(),
        );
    }
}

pub fn snapshot_current_process_runtime() -> Option<RunnerInfoResponse> {
    RuntimeSampler::new().snapshot()
}
