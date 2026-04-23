use std::collections::HashMap;
use std::time::Duration;

use crate::metrics::MetricsStore;

/// The result of a single scenario run.
///
/// Always contains `duration` and aggregated `metrics`. If a user step body
/// returned `Err`, `success` is `false` and `error` carries the error
/// message. Framework errors (hook failures, policy violations) do not
/// produce a `RunSummary` — they surface as `Err` from `Scenario::run`.
#[derive(Debug, Clone, PartialEq)]
pub struct RunSummary {
    pub success: bool,
    pub duration: Duration,
    pub error: Option<String>,
    pub metrics: MetricsSummary,
}

/// Aggregated metric data from a run.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct MetricsSummary {
    pub counters: HashMap<String, u64>,
    pub latencies: HashMap<String, LatencySummary>,
}

/// Summary of a single latency metric series.
#[derive(Debug, Clone, PartialEq)]
pub struct LatencySummary {
    pub count: u64,
    pub min: Option<Duration>,
    pub max: Option<Duration>,
    pub mean: Option<Duration>,
    pub p95: Option<Duration>,
    pub p99: Option<Duration>,
    pub throughput: f64,
}

impl LatencySummary {
    /// Summary for a metric name that received no samples during the run.
    fn empty() -> Self {
        Self {
            count: 0,
            min: None,
            max: None,
            mean: None,
            p95: None,
            p99: None,
            throughput: 0.0,
        }
    }
}

/// Pick the sample at `ceil(p * n) - 1`, clamped to `[0, n - 1]`.
///
/// Caller must ensure `sorted` is non-empty and in ascending order.
pub(crate) fn percentile(sorted: &[Duration], p: f64) -> Duration {
    debug_assert!(!sorted.is_empty());
    let n = sorted.len();
    let raw = (p * n as f64).ceil() as isize - 1;
    let idx = raw.max(0) as usize;
    let idx = idx.min(n - 1);
    sorted[idx]
}

/// Aggregate a single latency series over the total `run_duration`.
pub(crate) fn build_latency_summary(
    samples: &[Duration],
    run_duration: Duration,
) -> LatencySummary {
    if samples.is_empty() {
        return LatencySummary::empty();
    }

    let mut sorted: Vec<Duration> = samples.to_vec();
    sorted.sort();

    let count = sorted.len() as u64;
    let min = Some(*sorted.first().unwrap());
    let max = Some(*sorted.last().unwrap());

    // Accumulate in u128 nanoseconds to avoid float drift on the mean.
    let total_nanos: u128 = sorted.iter().map(|d| d.as_nanos()).sum();
    let mean_nanos = total_nanos / count as u128;
    let mean = Some(Duration::new(
        (mean_nanos / 1_000_000_000) as u64,
        (mean_nanos % 1_000_000_000) as u32,
    ));

    let p95 = Some(percentile(&sorted, 0.95));
    let p99 = Some(percentile(&sorted, 0.99));

    let throughput = if run_duration.is_zero() {
        0.0
    } else {
        count as f64 / run_duration.as_secs_f64()
    };

    LatencySummary {
        count,
        min,
        max,
        mean,
        p95,
        p99,
        throughput,
    }
}

pub(crate) fn build_metrics_summary(
    store: &MetricsStore,
    run_duration: Duration,
) -> MetricsSummary {
    let counters = store.counters.clone();
    let latencies = store
        .latencies
        .iter()
        .map(|(name, samples)| (name.clone(), build_latency_summary(samples, run_duration)))
        .collect();
    MetricsSummary {
        counters,
        latencies,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ms(n: u64) -> Duration {
        Duration::from_millis(n)
    }

    #[test]
    fn percentile_clamps_and_uses_ceil_rule() {
        let sorted: Vec<Duration> = (1..=5).map(ms).collect();
        // p=0.0 -> ceil(0) - 1 = -1 -> clamp to 0 -> 1ms
        assert_eq!(percentile(&sorted, 0.0), ms(1));
        // p=0.5 -> ceil(2.5) - 1 = 2 -> 3ms
        assert_eq!(percentile(&sorted, 0.5), ms(3));
        // p=0.95 -> ceil(4.75) - 1 = 4 -> 5ms
        assert_eq!(percentile(&sorted, 0.95), ms(5));
        // p=1.0 -> ceil(5) - 1 = 4 -> 5ms
        assert_eq!(percentile(&sorted, 1.0), ms(5));
    }

    #[test]
    fn build_latency_summary_known_samples() {
        let samples = vec![ms(50), ms(10), ms(30), ms(40), ms(20)];
        let s = build_latency_summary(&samples, Duration::from_secs(1));
        assert_eq!(s.count, 5);
        assert_eq!(s.min, Some(ms(10)));
        assert_eq!(s.max, Some(ms(50)));
        assert_eq!(s.mean, Some(ms(30)));
        // With 5 samples, both p95 and p99 land on the last element.
        assert_eq!(s.p95, Some(ms(50)));
        assert_eq!(s.p99, Some(ms(50)));
        assert!((s.throughput - 5.0).abs() < 1e-9);
    }

    #[test]
    fn build_latency_summary_empty_is_none_and_zero() {
        let s = build_latency_summary(&[], Duration::from_secs(1));
        assert_eq!(s.count, 0);
        assert!(s.min.is_none());
        assert!(s.max.is_none());
        assert!(s.mean.is_none());
        assert!(s.p95.is_none());
        assert!(s.p99.is_none());
        assert_eq!(s.throughput, 0.0);
    }

    #[test]
    fn throughput_uses_total_run_duration() {
        let samples = vec![ms(1); 10];
        let s = build_latency_summary(&samples, Duration::from_secs(2));
        assert!((s.throughput - 5.0).abs() < 1e-9);
    }

    #[test]
    fn throughput_zero_when_run_duration_is_zero() {
        let samples = vec![ms(1)];
        let s = build_latency_summary(&samples, Duration::ZERO);
        assert_eq!(s.throughput, 0.0);
    }

    #[test]
    fn build_metrics_summary_groups_by_name() {
        let mut store = MetricsStore::default();
        store.counters.insert("hits".to_string(), 5);
        store
            .latencies
            .insert("read".to_string(), vec![ms(1), ms(2), ms(3)]);
        store
            .latencies
            .insert("write".to_string(), vec![ms(10), ms(20)]);

        let s = build_metrics_summary(&store, Duration::from_secs(1));
        assert_eq!(s.counters.get("hits").copied(), Some(5));
        assert_eq!(s.latencies.get("read").unwrap().count, 3);
        assert_eq!(s.latencies.get("write").unwrap().count, 2);
    }
}
