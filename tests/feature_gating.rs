//! Guards for the crate's dependency discipline: the core (no features)
//! must stay a `thiserror`-only crate that other libraries can depend on
//! for free, and the heavy `DeciderBackend` must exist only behind the
//! `decider` feature.

use std::collections::BTreeSet;
use std::process::Command;
use std::sync::Arc;

use ta_ask::{ask, DecisionBackend, DecisionSchema, FixtureBackend};

/// Crates allowed in the core's normal (non-dev) dependency graph:
/// `thiserror` and its proc-macro stack. Anything else showing up here
/// means a required dependency was added, which every downstream library
/// would then inherit.
const CORE_ALLOWED: &[&str] = &[
    "ta-ask",
    "thiserror",
    "thiserror-impl",
    "proc-macro2",
    "quote",
    "syn",
    "unicode-ident",
];

fn dependency_names(features: &[&str]) -> BTreeSet<String> {
    let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".to_string());
    let manifest = concat!(env!("CARGO_MANIFEST_DIR"), "/Cargo.toml");
    let mut cmd = Command::new(&cargo);
    cmd.args([
        "tree",
        "--manifest-path",
        manifest,
        "-p",
        "ta-ask",
        "--no-default-features",
        "-e",
        "normal",
        "--prefix",
        "none",
        "--format",
        "{p}",
    ]);
    if !features.is_empty() {
        cmd.args(["--features", &features.join(",")]);
    }
    let output = cmd
        .output()
        .unwrap_or_else(|e| panic!("failed to run '{cargo} tree' for the dependency check: {e}"));
    assert!(
        output.status.success(),
        "'{cargo} tree' failed (features: {features:?}): {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter_map(|line| line.split_whitespace().next())
        .map(str::to_string)
        .collect()
}

#[test]
fn core_without_features_depends_on_thiserror_only() {
    let names = dependency_names(&[]);
    let unexpected: Vec<_> = names
        .iter()
        .filter(|n| !CORE_ALLOWED.contains(&n.as_str()))
        .collect();
    assert!(
        unexpected.is_empty(),
        "core ta-ask (no features) gained dependencies beyond thiserror: {unexpected:?}. \
         Make them optional and put them behind a feature."
    );
    assert!(names.contains("thiserror"));
}

#[test]
fn decider_feature_is_what_pulls_in_the_http_client() {
    let names = dependency_names(&["decider"]);
    assert!(
        names.contains("reqwest"),
        "the decider feature should enable reqwest, got: {names:?}"
    );
    assert!(
        !names.iter().any(|n| n.starts_with("pyo3")),
        "the core crate must never depend on pyo3, even with decider enabled"
    );
}

#[test]
fn core_api_is_usable_through_an_arc_dyn_backend() {
    // The shape downstream libraries use: hold any backend behind an Arc and
    // call ask() with it, without naming a concrete backend type.
    let backend: Arc<dyn DecisionBackend> = Arc::new(FixtureBackend::yes_no(false, 0.75));
    let response = ask(backend.as_ref(), "q", "c", DecisionSchema::YesNo).unwrap();
    assert_eq!(response.result, ta_ask::DecisionResult::Bool(false));
    assert_eq!(response.model_id, "fixture");
}

#[cfg(feature = "decider")]
#[test]
fn decider_feature_exports_decider_backend_as_a_decision_backend() {
    let backend: Arc<dyn DecisionBackend> = Arc::new(ta_ask::DeciderBackend::connect(
        "http://127.0.0.1:9",
        "decider-4b",
    ));
    // Score is rejected before any network call, so this needs no server.
    let err = ask(
        backend.as_ref(),
        "q",
        "c",
        DecisionSchema::Score { min: 0.0, max: 1.0 },
    )
    .unwrap_err();
    assert!(matches!(err, ta_ask::DecisionError::UnsupportedSchema(_)));
}
