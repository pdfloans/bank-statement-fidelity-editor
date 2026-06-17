use crate::app::config::AppConfig;
#[cfg(feature = "otel")]
use opentelemetry::KeyValue;
#[cfg(feature = "otel")]
use opentelemetry_otlp::WithExportConfig;
#[cfg(feature = "otel")]
use opentelemetry_sdk::trace;
#[cfg(feature = "otel")]
use opentelemetry_sdk::Resource;
use std::sync::Once;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt, EnvFilter, Layer};

static PANIC_HOOK: Once = Once::new();

pub struct TelemetryGuard {
    // Hold onto the guard for the non-blocking file appender if we ever use one
    // But tracing_appender::rolling doesn't strictly need a guard to stay alive
    // unlike the non_blocking one, however we return a guard for OTLP flush
}

impl Drop for TelemetryGuard {
    fn drop(&mut self) {
        #[cfg(feature = "otel")]
        opentelemetry::global::shutdown_tracer_provider();
    }
}

fn install_panic_hook() {
    PANIC_HOOK.call_once(|| {
        let default_hook = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            let payload = info
                .payload()
                .downcast_ref::<&'static str>()
                .map(|s| s.to_string())
                .or_else(|| info.payload().downcast_ref::<String>().cloned())
                .unwrap_or_else(|| "<unknown panic payload>".to_string());
            let location = info
                .location()
                .map(|l| format!("{}:{}", l.file(), l.line()))
                .unwrap_or_else(|| "<unknown location>".to_string());
            tracing::error!("[PANIC] at {} -- {}", location, payload);
            default_hook(info);
        }));
    });
}

pub fn init(cfg: &AppConfig) -> TelemetryGuard {
    // Best-effort log directory creation. Config loading already validates
    // this, but we re-check here and warn clearly if it regressed (e.g. the
    // directory was removed between config load and telemetry init).
    if let Err(e) = std::fs::create_dir_all(&cfg.log_dir) {
        eprintln!(
            "⚠️ Could not create log directory '{}': {}. File logging will be disabled; \
             logs will go to stdout only.",
            cfg.log_dir.display(),
            e
        );
    }
    install_panic_hook();

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

    let error_appender = tracing_appender::rolling::never("audit", "error_report.log");
    let error_layer = tracing_subscriber::fmt::layer()
        .with_writer(error_appender)
        .with_ansi(false)
        .with_thread_ids(true)
        .with_thread_names(true)
        .with_filter(EnvFilter::new("warn"));

    let subscriber = tracing_subscriber::registry()
        .with(env_filter)
        .with(stdout_layer)
        .with(file_layer)
        .with(error_layer);

    #[cfg(feature = "otel")]
    if let Some(endpoint) = &cfg.otel_endpoint {
        // OTLP requires a running tokio runtime — gracefully degrade if absent.
        let in_tokio = tokio::runtime::Handle::try_current().is_ok();
        if !in_tokio {
            eprintln!(
                "⚠️ OTLP endpoint '{endpoint}' configured but no Tokio runtime is available; skipping OTLP install."
            );
            subscriber.init();
            return TelemetryGuard {};
        }

        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            opentelemetry_otlp::new_pipeline()
                .tracing()
                .with_exporter(
                    opentelemetry_otlp::new_exporter()
                        .tonic()
                        .with_endpoint(endpoint),
                )
                .with_trace_config(trace::config().with_resource(Resource::new(vec![
                    KeyValue::new("service.name", cfg.otel_service_name.clone()),
                ])))
                .install_batch(opentelemetry_sdk::runtime::Tokio)
        }));

        match result {
            Ok(Ok(tracer)) => {
                let otel_layer = tracing_opentelemetry::layer().with_tracer(tracer);
                subscriber.with(otel_layer).init();
            }
            Ok(Err(e)) => {
                eprintln!(
                    "⚠️ Failed to initialize OTLP tracer at {endpoint}: {e}. Continuing without OTLP."
                );
                subscriber.init();
            }
            Err(_) => {
                eprintln!(
                    "⚠️ OTLP install panicked (likely missing runtime); continuing without OTLP."
                );
                subscriber.init();
            }
        }
    } else {
        subscriber.init();
    }

    // Recommendation #8: when built without the `otel` feature, there is no
    // exporter to install — just bring up stdout/file logging.
    #[cfg(not(feature = "otel"))]
    subscriber.init();

    TelemetryGuard {}
}

#[cfg(test)]
mod tests {
    #[cfg(feature = "otel")]
    use super::*;
    #[cfg(feature = "otel")]
    use std::path::PathBuf;

    #[cfg(feature = "otel")]
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
