//! Best-effort live spans. Neither network I/O nor a full queue can block
//! the Session's stream reader; shutdown has one bound for the whole queue.
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, SyncSender};
use std::time::Duration;

use crate::otlp_jsonl::Spans;

const REQUEST_TIMEOUT: Duration = Duration::from_millis(150);
const SHUTDOWN_TIMEOUT: Duration = Duration::from_millis(200);

pub(super) struct Export {
    pending: Option<SyncSender<Spans>>,
    done: Receiver<()>,
    stopped: Arc<AtomicBool>,
}

impl Export {
    pub(super) fn start(endpoint: String) -> Option<Self> {
        let (pending, requests) = mpsc::sync_channel::<Spans>(64);
        let (completed, done) = mpsc::channel();
        let stopped = Arc::new(AtomicBool::new(false));
        let stopping = Arc::clone(&stopped);
        std::thread::Builder::new()
            .name("otlp-export".into())
            .spawn(move || {
                let Ok(client) = reqwest::blocking::Client::builder()
                    .timeout(REQUEST_TIMEOUT)
                    .redirect(reqwest::redirect::Policy::none())
                    .build()
                else {
                    return;
                };
                for request in requests {
                    if stopping.load(Ordering::Relaxed) {
                        break;
                    }
                    // The codec's Spans is an OTLP ExportTraceServiceRequest.
                    // Failures, including HTTP errors, cost only this span.
                    let _ = client.post(&endpoint).json(&request).send();
                }
                drop(client);
                let _ = completed.send(());
            })
            .ok()?;
        Some(Self {
            pending: Some(pending),
            done,
            stopped,
        })
    }

    pub(super) fn send(&self, request: Spans) {
        if let Some(pending) = &self.pending {
            let _ = pending.try_send(request);
        }
    }
}

impl Drop for Export {
    fn drop(&mut self) {
        // Closing lets a healthy receiver drain even the final Session span.
        self.pending.take();
        let _ = self.done.recv_timeout(SHUTDOWN_TIMEOUT);
        // An unhealthy receiver must not drain the backlog after this bound.
        self.stopped.store(true, Ordering::Relaxed);
    }
}
