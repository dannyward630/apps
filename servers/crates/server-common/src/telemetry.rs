// SPDX-License-Identifier: FSL-1.1-ALv2
use opentelemetry::trace::TracerProvider;
use prometheus::{
    Encoder, HistogramOpts, HistogramVec, IntCounter, IntCounterVec, Opts, Registry, TextEncoder,
};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LogFormat {
    Text,
    Json,
}

#[derive(Clone, Copy)]
pub enum Service {
    DevBase,
    GitTracker,
}
pub struct Metrics {
    registry: Registry,
    pub requests: IntCounterVec,
    pub duration: HistogramVec,
    pub events: IntCounterVec,
    pub limits: IntCounterVec,
    pub cleanup_runs: IntCounter,
    pub cleanup_failures: IntCounter,
}
impl Metrics {
    pub fn new(service: Service) -> Self {
        let registry = Registry::new();
        let (name, labels) = match service {
            Service::DevBase => ("devbase_requests_total", vec!["procedure", "code"]),
            Service::GitTracker => ("git_tracker_http_requests_total", vec!["outcome"]),
        };
        let requests =
            IntCounterVec::new(Opts::new(name, "Requests by bounded outcome."), &labels).unwrap();
        let duration = HistogramVec::new(
            HistogramOpts::new(
                "devbase_request_duration_seconds",
                "Request latency by procedure.",
            ),
            &["procedure"],
        )
        .unwrap();
        let events = IntCounterVec::new(
            Opts::new("git_tracker_events_total", "Bounded lifecycle events."),
            &["kind", "category"],
        )
        .unwrap();
        let limits = IntCounterVec::new(
            Opts::new(
                "devbase_limit_rejections_total",
                "Rejections by limit kind.",
            ),
            &["kind"],
        )
        .unwrap();
        let cleanup_runs = IntCounter::new(
            "devbase_cleanup_runs_total",
            "Completed maintenance cycles.",
        )
        .unwrap();
        let cleanup_failures = IntCounter::new(
            "devbase_cleanup_failures_total",
            "Failed cleanup operations.",
        )
        .unwrap();
        registry.register(Box::new(requests.clone())).unwrap();
        match service {
            Service::DevBase => {
                registry.register(Box::new(duration.clone())).unwrap();
                registry.register(Box::new(limits.clone())).unwrap();
                registry.register(Box::new(cleanup_runs.clone())).unwrap();
                registry
                    .register(Box::new(cleanup_failures.clone()))
                    .unwrap();
            }
            Service::GitTracker => {
                registry.register(Box::new(events.clone())).unwrap();
                for labels in [
                    ["persistence", "failure"],
                    ["github", "failure"],
                    ["execution", "interrupted"],
                    ["webhook", "success"],
                    ["webhook", "rejected"],
                    ["webhook", "failure"],
                ] {
                    events.with_label_values(&labels);
                }
            }
        }
        Self {
            registry,
            requests,
            duration,
            events,
            limits,
            cleanup_runs,
            cleanup_failures,
        }
    }
    pub fn encode(&self) -> String {
        let mut out = Vec::new();
        TextEncoder::new()
            .encode(&self.registry.gather(), &mut out)
            .expect("encode registered metrics");
        String::from_utf8(out).unwrap()
    }
}
pub fn init() -> anyhow::Result<Option<opentelemetry_sdk::trace::SdkTracerProvider>> {
    init_with_default(LogFormat::Text)
}

pub fn init_with_default(
    default_format: LogFormat,
) -> anyhow::Result<Option<opentelemetry_sdk::trace::SdkTracerProvider>> {
    let format = select_format(std::env::var("LOG_FORMAT").ok().as_deref(), default_format)?;
    let provider = if std::env::var_os("OTEL_EXPORTER_OTLP_ENDPOINT").is_some()
        || std::env::var_os("OTEL_EXPORTER_OTLP_TRACES_ENDPOINT").is_some()
    {
        Some(
            opentelemetry_sdk::trace::SdkTracerProvider::builder()
                .with_batch_exporter(
                    opentelemetry_otlp::SpanExporter::builder()
                        .with_http()
                        .build()?,
                )
                .build(),
        )
    } else {
        None
    };
    let layer = provider
        .as_ref()
        .map(|p| tracing_opentelemetry::layer().with_tracer(p.tracer("delino-api")));
    let filter =
        tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into());
    let registry = tracing_subscriber::registry().with(filter).with(layer);
    match format {
        LogFormat::Json => registry
            .with(
                tracing_subscriber::fmt::layer()
                    .json()
                    .with_writer(std::io::stderr),
            )
            .init(),
        LogFormat::Text => registry
            .with(
                tracing_subscriber::fmt::layer()
                    .with_ansi(std::env::var("NO_COLOR").unwrap_or_default().is_empty())
                    .with_writer(std::io::stderr),
            )
            .init(),
    }
    Ok(provider)
}

fn select_format(value: Option<&str>, default_format: LogFormat) -> anyhow::Result<LogFormat> {
    match value {
        None | Some("") => Ok(default_format),
        Some("text") => Ok(LogFormat::Text),
        Some("json") => Ok(LogFormat::Json),
        Some(_) => anyhow::bail!("LOG_FORMAT must be text or json"),
    }
}

#[cfg(test)]
mod tests {
    use super::{LogFormat, Metrics, Service, select_format};

    fn event_samples(metrics: &Metrics) -> Vec<String> {
        metrics
            .encode()
            .lines()
            .filter(|line| line.starts_with("git_tracker_events_total{"))
            .map(str::to_owned)
            .collect()
    }

    #[test]
    fn git_tracker_initializes_alerted_and_webhook_event_series() {
        let metrics = Metrics::new(Service::GitTracker);
        let mut samples = event_samples(&metrics);
        samples.sort();
        assert_eq!(
            samples,
            [
                "git_tracker_events_total{category=\"failure\",kind=\"github\"} 0",
                "git_tracker_events_total{category=\"failure\",kind=\"persistence\"} 0",
                "git_tracker_events_total{category=\"failure\",kind=\"webhook\"} 0",
                "git_tracker_events_total{category=\"interrupted\",kind=\"execution\"} 0",
                "git_tracker_events_total{category=\"rejected\",kind=\"webhook\"} 0",
                "git_tracker_events_total{category=\"success\",kind=\"webhook\"} 0",
            ]
        );

        metrics
            .events
            .with_label_values(&["github", "failure"])
            .inc();
        let mut samples = event_samples(&metrics);
        samples.sort();
        assert_eq!(
            samples,
            [
                "git_tracker_events_total{category=\"failure\",kind=\"github\"} 1",
                "git_tracker_events_total{category=\"failure\",kind=\"persistence\"} 0",
                "git_tracker_events_total{category=\"failure\",kind=\"webhook\"} 0",
                "git_tracker_events_total{category=\"interrupted\",kind=\"execution\"} 0",
                "git_tracker_events_total{category=\"rejected\",kind=\"webhook\"} 0",
                "git_tracker_events_total{category=\"success\",kind=\"webhook\"} 0",
            ]
        );
    }

    #[test]
    fn devbase_metrics_do_not_export_git_tracker_event_series() {
        let metrics = Metrics::new(Service::DevBase);
        assert!(event_samples(&metrics).is_empty());
        assert!(metrics.encode().contains("devbase_cleanup_runs_total 0"));
    }

    #[test]
    fn selects_service_default_and_explicit_override() {
        assert_eq!(
            select_format(None, LogFormat::Json).unwrap(),
            LogFormat::Json
        );
        assert_eq!(
            select_format(Some(""), LogFormat::Json).unwrap(),
            LogFormat::Json
        );
        assert_eq!(
            select_format(None, LogFormat::Text).unwrap(),
            LogFormat::Text
        );
        assert_eq!(
            select_format(Some("text"), LogFormat::Json).unwrap(),
            LogFormat::Text
        );
        assert_eq!(
            select_format(Some("json"), LogFormat::Text).unwrap(),
            LogFormat::Json
        );
    }

    #[test]
    fn rejects_invalid_explicit_format() {
        let error = select_format(Some("xml"), LogFormat::Json).unwrap_err();
        assert_eq!(error.to_string(), "LOG_FORMAT must be text or json");
    }
}
