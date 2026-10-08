//! `ta-ask`: a narrow, bounded-question decision primitive.
//!
//! One bounded question plus context in, one calibrated answer out:
//! `ask(backend, question, context, schema) -> DecisionResponse { result, confidence, .. }`.
//! The crate stays deliberately narrow. Pre-filtering, arbitration across
//! several answers, and model hosting are the caller's concern; this crate
//! only defines the request/response shape, the pluggable
//! [`DecisionBackend`] trait, and a few backends.
//!
//! The core (everything in this module) depends on `thiserror` alone. The
//! real local-model backend, [`DeciderBackend`], lives behind the
//! off-by-default `decider` cargo feature so that crates which only want the
//! types and trait (to accept a backend from their own caller, say) pay
//! nothing for an HTTP client.
//!
//! The interface is synchronous by design: see the README's "Interface is
//! synchronous, not async" section.

// Compiles and runs the README's Rust examples as doctests, so they can't
// silently rot.
#[cfg(doctest)]
#[doc = include_str!("../README.md")]
struct ReadmeDoctests;

#[cfg(feature = "decider")]
pub mod decider_backend;
#[cfg(feature = "decider")]
pub use decider_backend::DeciderBackend;

#[derive(Debug, Clone, PartialEq)]
pub struct DecisionRequest {
    pub question: String,
    pub context: String,
    pub schema: DecisionSchema,
}

#[derive(Debug, Clone, PartialEq)]
pub enum DecisionSchema {
    YesNo,
    Choice(Vec<String>),
    Score { min: f64, max: f64 },
}

#[derive(Debug, Clone, PartialEq)]
pub enum DecisionResult {
    Bool(bool),
    Choice(String),
    Score(f64),
}

#[derive(Debug, Clone, PartialEq)]
pub struct DecisionResponse {
    pub result: DecisionResult,
    pub confidence: f64,
    pub model_id: String,
    pub latency_ms: u64,
}

/// Every variant states what happened, what was attempted, and what to do
/// about it.
///
/// The set of variants is the same whether or not the `decider` feature is
/// enabled (cargo features must be additive), which is why
/// [`DecisionError::RequestFailed`] carries a boxed error rather than a
/// concrete HTTP-client type.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum DecisionError {
    #[error("backend does not support schema {0:?} yet -- use YesNo or Choice instead")]
    UnsupportedSchema(DecisionSchema),
    #[error(
        "failed to spawn backend server (command: '{command}'): {source} -- is the Python \
         environment set up? see the DeciderBackend section of the ta-ask README"
    )]
    SpawnFailed {
        command: String,
        #[source]
        source: std::io::Error,
    },
    #[error(
        "backend server at {url} did not become healthy within {timeout_secs}s -- is the \
         Python environment set up and the model downloaded? see the DeciderBackend section \
         of the ta-ask README"
    )]
    BackendUnavailable { url: String, timeout_secs: u64 },
    #[error(
        "backend server process exited early (status: {exit_status}) before becoming healthy -- \
         recent stderr: {stderr_tail}"
    )]
    ServerExitedEarly {
        exit_status: String,
        stderr_tail: String,
    },
    #[error("request to backend failed: {0}")]
    RequestFailed(#[source] Box<dyn std::error::Error + Send + Sync + 'static>),
    #[error("backend returned a malformed response: {0}")]
    MalformedResponse(String),
    /// A backend implementation's own failure that fits no variant above
    /// (for example, a user-supplied backend that raised). The message
    /// should say what the backend was doing when it failed.
    #[error("backend failed: {0}")]
    BackendFailed(String),
}

/// Pluggable decision-answering backend. Implementations must be safe to
/// share across threads (`Send + Sync`) since a single backend instance is
/// typically held by an `Arc` and reused across many `ask()` calls.
pub trait DecisionBackend: Send + Sync {
    fn decide(&self, req: &DecisionRequest) -> Result<DecisionResponse, DecisionError>;
}

/// Ask `backend` a single bounded question. Thin by design: constructs a
/// `DecisionRequest` and calls straight through to `backend.decide()`. All
/// the real logic lives in the backend implementation.
pub fn ask(
    backend: &dyn DecisionBackend,
    question: impl Into<String>,
    context: impl Into<String>,
    schema: DecisionSchema,
) -> Result<DecisionResponse, DecisionError> {
    let req = DecisionRequest {
        question: question.into(),
        context: context.into(),
        schema,
    };
    backend.decide(&req)
}

/// Deterministic backend returning a pre-configured response, for tests
/// that don't need (and must not require) a real model or network call.
pub struct FixtureBackend {
    response: DecisionResponse,
}

impl FixtureBackend {
    pub fn new(response: DecisionResponse) -> Self {
        Self { response }
    }

    /// Convenience constructor for the common yes/no case.
    pub fn yes_no(result: bool, confidence: f64) -> Self {
        Self::new(DecisionResponse {
            result: DecisionResult::Bool(result),
            confidence,
            model_id: "fixture".to_string(),
            latency_ms: 0,
        })
    }
}

impl DecisionBackend for FixtureBackend {
    fn decide(&self, _req: &DecisionRequest) -> Result<DecisionResponse, DecisionError> {
        Ok(self.response.clone())
    }
}

/// Deterministic backend that always errors with a configured message, for
/// testing a caller's error-handling path without a real failing backend.
pub struct ErrorFixtureBackend {
    error_message: String,
}

impl ErrorFixtureBackend {
    pub fn new(error_message: impl Into<String>) -> Self {
        Self {
            error_message: error_message.into(),
        }
    }
}

impl DecisionBackend for ErrorFixtureBackend {
    fn decide(&self, _req: &DecisionRequest) -> Result<DecisionResponse, DecisionError> {
        Err(DecisionError::MalformedResponse(self.error_message.clone()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fixture_backend_returns_configured_response() {
        let backend = FixtureBackend::yes_no(true, 0.9);
        let response = ask(
            &backend,
            "is this safe?",
            "some context",
            DecisionSchema::YesNo,
        )
        .unwrap();
        assert_eq!(response.result, DecisionResult::Bool(true));
        assert_eq!(response.confidence, 0.9);
    }

    #[test]
    fn error_fixture_backend_returns_configured_error() {
        let backend = ErrorFixtureBackend::new("boom");
        let err = ask(&backend, "q", "c", DecisionSchema::YesNo).unwrap_err();
        assert!(matches!(err, DecisionError::MalformedResponse(ref msg) if msg == "boom"));
    }

    #[test]
    fn ask_builds_the_request_the_backend_receives() {
        struct CapturingBackend {
            captured: std::sync::Mutex<Option<DecisionRequest>>,
        }
        impl DecisionBackend for CapturingBackend {
            fn decide(&self, req: &DecisionRequest) -> Result<DecisionResponse, DecisionError> {
                *self.captured.lock().unwrap() = Some(req.clone());
                Ok(DecisionResponse {
                    result: DecisionResult::Bool(true),
                    confidence: 1.0,
                    model_id: "capturing".to_string(),
                    latency_ms: 0,
                })
            }
        }
        let backend = CapturingBackend {
            captured: std::sync::Mutex::new(None),
        };
        ask(
            &backend,
            "question text",
            "context text",
            DecisionSchema::Choice(vec!["a".to_string(), "b".to_string()]),
        )
        .unwrap();
        let captured = backend.captured.lock().unwrap().clone().unwrap();
        assert_eq!(captured.question, "question text");
        assert_eq!(captured.context, "context text");
        assert_eq!(
            captured.schema,
            DecisionSchema::Choice(vec!["a".to_string(), "b".to_string()])
        );
    }

    #[test]
    fn request_failed_wraps_any_error_without_a_feature_specific_type() {
        let io = std::io::Error::new(std::io::ErrorKind::ConnectionRefused, "refused");
        let err = DecisionError::RequestFailed(Box::new(io));
        assert!(err.to_string().contains("refused"));
        assert!(std::error::Error::source(&err).is_some());
    }
}
