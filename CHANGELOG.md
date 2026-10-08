# Changelog

All notable changes to this project are documented here. Versions follow
[Semantic Versioning](https://semver.org/); while the version is `0.x`, a
minor bump may contain breaking changes.

## [0.1.0] - Unreleased

First standalone release, extracted from TrustedAutonomy's in-tree
`crates/ta-ask` at commit `71772bccab`.

### Added

- `python/` workspace member (`typed-ask-py`): a PyO3 + maturin extension
  module named `typed_ask`, built as an abi3 wheel (one wheel per platform for
  CPython 3.10+). Exposes the request/response types, `ask()`, the fixture
  backends, `DeciderBackend` (when built with the `decider` feature, which
  the published wheel enables), and any Python callable as a backend.
- `DecisionError::BackendFailed(String)` for a custom backend's own failure
  that fits no other variant (used by the Python callable backend).
- `tests/feature_gating.rs`, which fails if the core crate gains a required
  dependency beyond `thiserror`.
- README examples are compiled and run as doctests.

### Changed (relative to the in-tree TrustedAutonomy crate)

- `DeciderBackend` and its dependencies (`reqwest`, `serde`, `serde_json`)
  moved behind the off-by-default `decider` cargo feature. The core crate
  now depends on `thiserror` only.
- The `decider` feature builds `reqwest` with `default-features = false`
  (blocking + json only, no TLS stack). The server it talks to is local.
- `DecisionError::RequestFailed` now wraps
  `Box<dyn std::error::Error + Send + Sync>` instead of `reqwest::Error`, so
  the enum is identical with or without the `decider` feature.
- `DecisionError` is `#[non_exhaustive]`.
- `DeciderBackend`'s tests use a small std-only in-process HTTP server
  instead of `wiremock` + `tokio`, so testing pulls in no async runtime.
  Added tests for a "no" answer's derived confidence, an HTTP error status,
  and a connection failure.
- Error messages and docs no longer point at TrustedAutonomy-internal paths.
