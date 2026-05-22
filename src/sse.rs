use crate::models::*;
use std::sync::Arc;
use tokio::sync::{broadcast, RwLock};

/// Broadcaster for SSE events. Each event is JSON-serialized and sent to all connected clients.
#[derive(Clone)]
pub struct Broadcaster {
    tx: broadcast::Sender<String>,
    #[allow(dead_code)]
    rx: Arc<RwLock<broadcast::Receiver<String>>>,
}

impl Broadcaster {
    pub fn new() -> Self {
        let (tx, rx) = broadcast::channel(128);
        Self {
            tx,
            rx: Arc::new(RwLock::new(rx)),
        }
    }

    pub fn subscribe(&self) -> broadcast::Receiver<String> {
        self.tx.subscribe()
    }

    pub fn broadcast(&self, event: &SseEvent) {
        let json = match serde_json::to_string(event) {
            Ok(s) => s,
            Err(e) => {
                tracing::error!("failed to serialize SSE event: {}", e);
                return;
            }
        };
        let _ = self.tx.send(json);
    }
}

impl Default for Broadcaster {
    fn default() -> Self {
        Self::new()
    }
}
