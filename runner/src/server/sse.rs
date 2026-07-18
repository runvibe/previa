use serde_json::Value;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

#[derive(Debug)]
pub struct SseMessage {
    pub event: &'static str,
    pub data: Value,
}

pub fn send_sse_or_cancel(
    tx: &mpsc::UnboundedSender<SseMessage>,
    event: &'static str,
    data: Value,
    cancel: &CancellationToken,
) -> bool {
    if tx.send(SseMessage { event, data }).is_err() {
        cancel.cancel();
        return false;
    }
    true
}
