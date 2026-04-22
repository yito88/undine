use std::collections::HashMap;
use std::time::Duration;

use tokio::sync::mpsc::{self, UnboundedReceiver, UnboundedSender};

/// A metric or event emitted by workers or hooks during scenario execution.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MetricEvent {
    Counter { name: String, value: u64 },
    Latency { name: String, value: Duration },
    Event { name: String, message: String },
}

/// Lightweight, cloneable sink used by workers and hooks to emit events.
///
/// Backed by an unbounded channel owned by the running [`Scenario`].
#[derive(Clone)]
pub struct MetricsSink {
    sender: UnboundedSender<MetricEvent>,
}

impl MetricsSink {
    pub(crate) fn new() -> (Self, UnboundedReceiver<MetricEvent>) {
        let (tx, rx) = mpsc::unbounded_channel();
        (Self { sender: tx }, rx)
    }

    pub fn emit(&self, event: MetricEvent) {
        let _ = self.sender.send(event);
    }

    pub fn counter(&self, name: impl Into<String>, value: u64) {
        self.emit(MetricEvent::Counter {
            name: name.into(),
            value,
        });
    }

    pub fn latency(&self, name: impl Into<String>, value: Duration) {
        self.emit(MetricEvent::Latency {
            name: name.into(),
            value,
        });
    }

    pub fn event(&self, name: impl Into<String>, message: impl Into<String>) {
        self.emit(MetricEvent::Event {
            name: name.into(),
            message: message.into(),
        });
    }
}

/// Aggregate of metric events collected after scenario execution.
///
/// This is kept internal for now; no public reporting API is exposed yet.
#[derive(Debug, Default)]
pub(crate) struct MetricsStore {
    pub counters: HashMap<String, u64>,
    pub latencies: HashMap<String, Vec<Duration>>,
    pub events: Vec<MetricEvent>,
}

impl MetricsStore {
    fn ingest(&mut self, event: MetricEvent) {
        match &event {
            MetricEvent::Counter { name, value } => {
                *self.counters.entry(name.clone()).or_insert(0) += value;
            }
            MetricEvent::Latency { name, value } => {
                self.latencies
                    .entry(name.clone())
                    .or_default()
                    .push(*value);
            }
            MetricEvent::Event { .. } => {}
        }
        self.events.push(event);
    }

    pub(crate) fn drain_from(rx: &mut UnboundedReceiver<MetricEvent>) -> Self {
        let mut store = Self::default();
        while let Ok(event) = rx.try_recv() {
            store.ingest(event);
        }
        store
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn sink_emits_counters_and_drain_aggregates() {
        let (sink, mut rx) = MetricsSink::new();
        sink.counter("hits", 1);
        sink.counter("hits", 4);
        sink.counter("misses", 2);

        drop(sink);
        let store = MetricsStore::drain_from(&mut rx);
        assert_eq!(store.counters.get("hits").copied(), Some(5));
        assert_eq!(store.counters.get("misses").copied(), Some(2));
    }

    #[tokio::test]
    async fn sink_emits_latencies_and_events() {
        let (sink, mut rx) = MetricsSink::new();
        sink.latency("req", Duration::from_millis(5));
        sink.latency("req", Duration::from_millis(7));
        sink.event("note", "hello");

        drop(sink);
        let store = MetricsStore::drain_from(&mut rx);
        assert_eq!(store.latencies.get("req").map(|v| v.len()), Some(2));
        assert_eq!(store.events.len(), 3);
    }
}
