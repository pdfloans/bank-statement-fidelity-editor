use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt, EnvFilter};
use crate::app::config::AppConfig;
use opentelemetry_sdk::trace;
use opentelemetry_sdk::Resource;
use opentelemetry::KeyValue;
use opentelemetry_otlp::WithExportConfig;

pub struct TelemetryGuard {
    // Hold onto the guard for the non-blocking file appender if we ever use one
    // But tracing_appender::rolling doesn't strictly need a guard to stay alive
    // unlike the non_blocking one, however we return a guard for OTLP flush
}

impl Drop for TelemetryGuard {
    fn drop(&mut self) {
        opentelemetry::global::shutdown_tracer_provider();
    }
}

pub fn init(cfg: &AppConfig) -> TelemetryGuard {
    std::fs::create_dir_all(&cfg.log_dir).ok();

    let env_filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));

    let stdout_layer = tracing_subscriber::fmt::layer()
        .with_target(false)
        .with_thread_ids(true)
        .with_thread_names(true);

    let file_appender = tracing_appender::rolling::daily(&cfg.log_dir, "app.log");
    let file_layer = tracing_subscriber::fmt::layer()
        .with_writer(file_appender)
        .with_ansi(false)
        .with_thread_ids(true)
        .with_thread_names(true);

    let mut subscriber = tracing_subscriber::registry()
        .with(env_filter)
        .with(stdout_layer)
        .with(file_layer);

    if let Some(endpoint) = &cfg.otel_endpoint {
        let result = opentelemetry_otlp::new_pipeline()
            .tracing()
            .with_exporter(
                opentelemetry_otlp::new_exporter()
                    .tonic()
                    .with_endpoint(endpoint),
            )
            .with_trace_config(
                trace::config().with_resource(Resource::new(vec![KeyValue::new(
                    "service.name",
                    cfg.otel_service_name.clone(),
                )])),
            )
            .install_batch(opentelemetry_sdk::runtime::Tokio);

        match result {
            Ok(tracer) => {
                let otel_layer = tracing_opentelemetry::layer().with_tracer(tracer);
                subscriber.with(otel_layer).init();
            }
            Err(e) => {
                eprintln!("⚠️ Failed to initialize OTLP tracer at {}: {}. Continuing without OTLP.", endpoint, e);
                subscriber.init();
            }
        }
    } else {
        subscriber.init();
    }

    TelemetryGuard {}
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn init_with_unreachable_otlp_endpoint_does_not_panic() {
        let cfg = AppConfig {
            log_dir: PathBuf::from("logs_test"),
            otel_endpoint: Some("http://127.0.0.1:1".into()),
            otel_service_name: "test-service".into(),
            ..AppConfig::default()
        };
        // Should not panic
        let _guard = init(&cfg);
    }
}
