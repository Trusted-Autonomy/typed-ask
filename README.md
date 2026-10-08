# ta-ask

**A narrow, bounded-question decision primitive: `ask(question, context, schema) -> {result, confidence}`, with pluggable backends.**

A surprising number of places in an agent system want the exact same shape: one bounded question plus a bounded piece of context in, one calibrated answer out. "Does this diff look safe to commit?" "Is this message read-only chat, or does it need real dispatch?" "Which department should handle this ticket?" "Do these two tasks overlap?" Each one could grow its own prompt, its own model client, its own confidence parsing and its own test doubles, and then they all drift apart. This crate is the one shared primitive instead: a request/response shape, a pluggable backend trait, deterministic test backends, and one real local-model backend.

It stays deliberately narrow. Pre-filtering (regex, BM25), arbitration across several answers, and model hosting are the caller's concern. Arbitration in particular should be built as a *composition* of narrow `ask()` calls combined by deterministic logic, not as one bigger holistic call.

## What it does

```rust
use ta_ask::{ask, DecisionResult, DecisionSchema, FixtureBackend};

// Any `DecisionBackend` works here; FixtureBackend is deterministic, for tests.
let backend = FixtureBackend::yes_no(true, 0.93);

let response = ask(
    &backend,
    "Does this diff look safe to commit?",
    "3 files changed, 12 lines, tests updated",
    DecisionSchema::YesNo,
)?;

assert_eq!(response.result, DecisionResult::Bool(true));
assert!(response.confidence >= 0.9);
println!("answered by {} in {}ms", response.model_id, response.latency_ms);
# Ok::<(), ta_ask::DecisionError>(())
```

## Types

| Type | Shape |
|---|---|
| `DecisionRequest` | `{ question, context, schema }` |
| `DecisionSchema` | `YesNo` \| `Choice(Vec<String>)` \| `Score { min, max }` |
| `DecisionResponse` | `{ result, confidence, model_id, latency_ms }` |
| `DecisionResult` | `Bool(bool)` \| `Choice(String)` \| `Score(f64)`, matching the schema |
| `DecisionError` | what happened, what was attempted, and what to do about it |

`confidence` is a calibrated `0.0..=1.0`. `model_id` records which backend actually answered, so production usage stays comparable across model choices, not just in offline evaluation.

## Bring your own backend

```rust
use ta_ask::{DecisionBackend, DecisionError, DecisionRequest, DecisionResponse, DecisionResult};

struct AlwaysBilling;

impl DecisionBackend for AlwaysBilling {
    fn decide(&self, _req: &DecisionRequest) -> Result<DecisionResponse, DecisionError> {
        Ok(DecisionResponse {
            result: DecisionResult::Choice("billing".into()),
            confidence: 1.0,
            model_id: "rule:always-billing".into(),
            latency_ms: 0,
        })
    }
}
```

Backends must be `Send + Sync`; a single instance is typically held in an `Arc<dyn DecisionBackend>` and reused across many calls.

## Interface is synchronous, not async

`DecisionBackend::decide` is a plain sync function, not `async fn`. This was a deliberate correction from the original design sketch: the first real consumer (a reviewer node in a sync workflow-graph engine) has no async anywhere, and making this sync means zero runtime-bridging code at the integration point. A future async caller can wrap a sync `ask()` call in `tokio::task::spawn_blocking`, which is the standard, safe direction for calling blocking code from async Rust (the reverse, sync code calling async, is the fragile direction).

## Features and dependencies

The core (types, trait, `ask()`, fixture backends) depends on `thiserror` and nothing else, so other libraries can accept a `DecisionBackend` from their caller without inheriting an HTTP client. A test in `tests/feature_gating.rs` enforces this.

| Feature | Default | Adds |
|---|---|---|
| `decider` | off | `DeciderBackend`, plus `reqwest` (blocking, no TLS), `serde`, `serde_json` |

```toml
[dependencies]
ta-ask = { git = "https://github.com/Trusted-Autonomy/ta-ask", tag = "v0.1.0" }
# or, with the real local-model backend:
ta-ask = { git = "https://github.com/Trusted-Autonomy/ta-ask", tag = "v0.1.0", features = ["decider"] }
```

## Python

The same repo ships a Python package, `ta-ask` on the index and `ta_ask` on import, built from the `python/` workspace member with PyO3 and maturin as one abi3 wheel per platform (CPython 3.10+). It is a thin layer over this crate: same types, same backends, same errors. `pyo3` is a dependency of `python/` only, never of the core crate, so `cargo test` here needs no Python interpreter.

```python
import ta_ask

response = ta_ask.ask(
    ta_ask.FixtureBackend.yes_no(True, 0.93),
    "Does this diff look safe to commit?",
    "3 files changed, 12 lines, tests updated",
    ta_ask.DecisionSchema.yes_no(),
)
assert response.result is True  # bool for yes_no, str for choice, float for score

# Any callable fn(request) -> DecisionResponse is a backend:
def route(request: ta_ask.DecisionRequest) -> ta_ask.DecisionResponse:
    label = "billing" if "charged" in request.context else "technical"
    return ta_ask.DecisionResponse(label, 0.8, model_id="rule:keyword-router")

schema = ta_ask.DecisionSchema.choice(["billing", "technical", "sales"])
ta_ask.ask(route, "Which department should handle this?", "charged twice", schema).result
# 'billing'

# The real local-model backend (included in the published wheel):
backend = ta_ask.DeciderBackend.connect("http://127.0.0.1:8700", "decider-4b")
```

`ask()` releases the GIL while a Rust backend works. Errors are raised as `ta_ask.DecisionError` subclasses (`UnsupportedSchemaError`, `RequestFailedError`, `MalformedResponseError`, ...); an exception raised by a Python callable backend propagates unchanged. Build locally with `cd python && maturin develop`; see `python/README.md`.

## Backends

- `FixtureBackend` / `ErrorFixtureBackend`: deterministic, for tests. No network, no subprocess.
- `DeciderBackend` (feature `decider`): the real backend, wrapping Decider-4b (Mapika, Apache 2.0, github.com/Mapika/decider). Spawns and keeps warm Decider's own HTTP server (`scripts/serve.sh <model> <port>`) rather than a custom stdio protocol. The model takes 13-21s to load, so a long-lived process is required; Decider ships `/health` and `/decide` endpoints specifically for this kind of integration.

### Running `DeciderBackend` for real

Install dependencies (note: **two extras are required**: `[metal]` for Apple Silicon acceleration and `[serve]` for the FastAPI/uvicorn HTTP server):

```bash
python3 -m venv /tmp/ta-ask-decider-env
source /tmp/ta-ask-decider-env/bin/activate
pip install 'decider-ai[metal,serve]'   # [metal] is the Apple Silicon MPS acceleration extra
                                        # [serve] brings in fastapi/uvicorn/httpx
git clone https://github.com/Mapika/decider /tmp/decider-repo
```

Start the server (note: model id must be the **full Hugging Face repo id**, and UVICORN must point to the venv's uvicorn binary):

```bash
cd /tmp/decider-repo
UVICORN=/tmp/ta-ask-decider-env/bin/uvicorn \
PATH="/tmp/ta-ask-decider-env/bin:$PATH" \
bash scripts/serve.sh Mapika/decider-4b 8700
```

Then `DeciderBackend::connect("http://127.0.0.1:8700", "decider-4b")`, or `DeciderBackend::spawn(...)` to have Rust own the process lifecycle itself (the child is killed when the backend is dropped).

**`model_id` is load-bearing for `spawn()`, not just a label.** `connect()`'s `model_id` is a free-text label only (it never leaves the process; it's just copied into `DecisionResponse::model_id`). `spawn()`'s `model_id`, by contrast, is forwarded directly as an argument to `scripts/serve.sh`, which passes it straight to `decider.serve` as `DECIDER_MODEL`. It must be the full Hugging Face repo id (e.g. `"Mapika/decider-4b"`, not `"decider-4b"`), or the server fails to load the model.

**`spawn()` takes an `env` parameter** (`&[(&str, &str)]`) that's passed straight through to the child process's environment. The typical use is `UVICORN`, which `scripts/serve.sh` reads to locate the `uvicorn` binary (it defaults to a repo-relative `.venv312/bin/uvicorn`, which won't exist outside the original authors' own checkout):

```rust,ignore
DeciderBackend::spawn(
    Path::new("/tmp/decider-repo/scripts/serve.sh"),
    "Mapika/decider-4b",
    8700,
    &[("UVICORN", "/tmp/ta-ask-decider-env/bin/uvicorn")],
    Duration::from_secs(60),
)?;
```

**Plain HTTP only.** The `decider` feature builds `reqwest` without a TLS stack, because the server it talks to is a local process on `127.0.0.1`. An `https://` base URL will fail with `DecisionError::RequestFailed`.

**Wire format:** The real `/decide` endpoint takes and returns JSON objects, not arrays:
- Request: `{"context": "<str>", "schema": {"<question text>": {"type": "bool"} | {"type": "choice", "options": [...]}}}`
- Response (bool question): `{"<question text>": {"noul": <0..1 prob-of-yes>, "type": "noul"}}`
  - Confidence is synthesized from the `noul` probability: `confidence = if noul > 0.5 { noul } else { 1.0 - noul }`
- Response (choice question): `{"<question text>": {"choice": "<str>", "confidence": <0..1>, "type": "choice", "probabilities": {...}}}`

The tests that need a live Decider server are `#[ignore]`d; run them with `cargo test --features decider -- --ignored` once a server is up.

## Honest limits

- **`DecisionSchema::Score` is not supported by `DeciderBackend`.** It returns `DecisionError::UnsupportedSchema` before any HTTP call. No known consumer needs it yet. Decider does have a richer `system_one` API that supports scored answers with discrete levels, not a continuous `min..max` range; if `Score` support is ever needed, that's the API to map against, not a client-side approximation layered on `decide()`.
- **`ask()` does not validate the backend's answer against the schema.** A backend returning `Choice("x")` for a `YesNo` question is passed through as-is. Callers that need that guarantee should check `result` themselves.
- **`DeciderBackend` makes no claim of Windows support.** Decider's own `scripts/serve.sh` is Unix-only. The crate builds and its tests pass on Windows (fixtures use `.bat` scripts), but running a real Decider server there is untested.
- **One question per call.** A backend is free to batch several in-flight requests internally (Decider can answer several questions about one context in one forward pass), but the public interface stays single-question.

## Provenance

Extracted from [TrustedAutonomy](https://github.com/Trusted-Autonomy/TrustedAutonomy)'s `crates/ta-ask` at commit `71772bccab`. See `CHANGELOG.md` for what changed in the extraction.

## License

Apache-2.0
