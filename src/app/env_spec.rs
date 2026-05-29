//! Environment Variable Specification
//!
//! A single source of truth describing every environment variable the
//! application reads. Both the `doctor` diagnostics command and the
//! configuration-error messages draw from this module so that setup
//! guidance stays consistent and actionable.

/// Whether a variable is required, recommended, or optional.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Requirement {
    /// The application will not start without this.
    Required,
    /// Strongly recommended; some features are unavailable without it.
    Recommended,
    /// Purely optional; sensible defaults apply.
    Optional,
}

impl Requirement {
    pub fn label(&self) -> &'static str {
        match self {
            Requirement::Required => "REQUIRED",
            Requirement::Recommended => "RECOMMENDED",
            Requirement::Optional => "OPTIONAL",
        }
    }
}

/// A single environment variable's specification.
#[derive(Debug, Clone, Copy)]
pub struct EnvVarSpec {
    /// The environment variable name.
    pub name: &'static str,
    /// How critical the variable is.
    pub requirement: Requirement,
    /// One-line summary of what the variable controls.
    pub summary: &'static str,
    /// Which feature(s) become available when set.
    pub enables: &'static str,
    /// A short hint on how to obtain or choose a value.
    pub setup_hint: &'static str,
    /// An example or placeholder value (never a real secret).
    pub example: &'static str,
}

/// The complete catalogue of environment variables.
pub const ENV_VARS: &[EnvVarSpec] = &[
    EnvVarSpec {
        name: "DUAL_CORE_PASSPHRASE",
        requirement: Requirement::Required,
        summary: "Passphrase that unlocks the application.",
        enables: "Application startup",
        setup_hint: "Choose a strong secret of at least 16 characters (8 in dev builds).",
        example: "a-long-random-passphrase-1234",
    },
    EnvVarSpec {
        name: "PYMUPDF_PRO_KEY",
        requirement: Requirement::Required,
        summary: "PyMuPDF Pro license key.",
        enables: "PDF parsing/rendering engine",
        setup_hint: "Obtain from https://pymupdf.io/ and keep it out of version control.",
        example: "your_pymupdf_pro_key_here",
    },
    EnvVarSpec {
        name: "GEMINI_API_KEY",
        requirement: Requirement::Recommended,
        summary: "Google Gemini API key.",
        enables: "Smart Balance Engine (AI proposals)",
        setup_hint: "Create one at https://aistudio.google.com/app/apikey",
        example: "your_gemini_api_key",
    },
    EnvVarSpec {
        name: "DOCUMENT_AI_PROJECT_ID",
        requirement: Requirement::Recommended,
        summary: "Google Cloud project ID for Document AI.",
        enables: "Transaction extraction (Extract / Balance)",
        setup_hint: "Found in the Google Cloud Console project picker.",
        example: "my-gcp-project",
    },
    EnvVarSpec {
        name: "DOCUMENT_AI_LOCATION",
        requirement: Requirement::Recommended,
        summary: "Document AI processor region.",
        enables: "Transaction extraction (Extract / Balance)",
        setup_hint: "Typically 'us' or 'eu' — must match your processor.",
        example: "us",
    },
    EnvVarSpec {
        name: "DOCUMENT_AI_PROCESSOR_ID",
        requirement: Requirement::Recommended,
        summary: "Document AI processor ID.",
        enables: "Transaction extraction (Extract / Balance)",
        setup_hint: "Copy from the processor's detail page in the Console.",
        example: "abcdef1234567890",
    },
    EnvVarSpec {
        name: "DOCUMENT_AI_API_KEY",
        requirement: Requirement::Optional,
        summary: "Document AI API key (Beta v1beta3).",
        enables: "Preferred Document AI auth (over OAuth/service account)",
        setup_hint: "Create at https://console.cloud.google.com/apis/credentials",
        example: "",
    },
    EnvVarSpec {
        name: "GOOGLE_APPLICATION_CREDENTIALS",
        requirement: Requirement::Optional,
        summary: "Path to a service-account JSON key.",
        enables: "Document AI auth fallback (legacy)",
        setup_hint: "Not needed if you ran `gcloud auth application-default login`.",
        example: "/path/to/service-account.json",
    },
    EnvVarSpec {
        name: "PDFREST_API_KEY",
        requirement: Requirement::Optional,
        summary: "Adobe pdfRest API key.",
        enables: "Higher-tier visual verification rendering",
        setup_hint: "Get one at https://pdfrest.com/ — falls back to local rendering when absent.",
        example: "",
    },
    EnvVarSpec {
        name: "OTEL_EXPORTER_OTLP_ENDPOINT",
        requirement: Requirement::Optional,
        summary: "OpenTelemetry OTLP gRPC endpoint.",
        enables: "Distributed tracing export",
        setup_hint: "Point at your collector, e.g. http://localhost:4317",
        example: "http://localhost:4317",
    },
    EnvVarSpec {
        name: "OTEL_SERVICE_NAME",
        requirement: Requirement::Optional,
        summary: "Service name reported in telemetry.",
        enables: "Telemetry labelling",
        setup_hint: "Defaults to 'dual-core-pdf-pipeline'.",
        example: "dual-core-pdf-pipeline",
    },
    EnvVarSpec {
        name: "LOG_DIR",
        requirement: Requirement::Optional,
        summary: "Directory for rotating log files.",
        enables: "File-based logging",
        setup_hint: "Defaults to './logs'; must be writable.",
        example: "./logs",
    },
    EnvVarSpec {
        name: "RUST_LOG",
        requirement: Requirement::Optional,
        summary: "Log verbosity filter.",
        enables: "Console/file log level control",
        setup_hint: "e.g. 'info', 'debug', or 'dual_core_pdf_pipeline=debug'.",
        example: "info",
    },
];

/// Look up a single variable's spec by name.
pub fn lookup(name: &str) -> Option<&'static EnvVarSpec> {
    ENV_VARS.iter().find(|v| v.name == name)
}

/// Build a detailed, multi-line setup-guidance message for a single
/// variable. Used by configuration-error reporting so a failure tells the
/// user exactly how to fix it.
pub fn guidance_for(name: &str) -> String {
    match lookup(name) {
        Some(spec) => format!(
            "{} ({})\n  Purpose : {}\n  Enables : {}\n  Setup   : {}\n  Example : {}={}",
            spec.name,
            spec.requirement.label(),
            spec.summary,
            spec.enables,
            spec.setup_hint,
            spec.name,
            spec.example,
        ),
        None => format!("{}: no setup guidance available.", name),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_required_var_has_setup_hint() {
        for spec in ENV_VARS {
            assert!(
                !spec.setup_hint.is_empty(),
                "{} is missing a setup hint",
                spec.name
            );
            assert!(!spec.summary.is_empty(), "{} is missing a summary", spec.name);
        }
    }

    #[test]
    fn lookup_finds_known_var() {
        assert!(lookup("DUAL_CORE_PASSPHRASE").is_some());
        assert!(lookup("NONEXISTENT_VAR").is_none());
    }

    #[test]
    fn guidance_mentions_the_variable_name() {
        let g = guidance_for("GEMINI_API_KEY");
        assert!(g.contains("GEMINI_API_KEY"));
        assert!(g.contains("Setup"));
    }

    #[test]
    fn guidance_for_unknown_var_is_graceful() {
        let g = guidance_for("MADE_UP");
        assert!(g.contains("no setup guidance"));
    }
}
