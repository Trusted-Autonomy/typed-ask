# typed-ask (Python)

Python bindings for [typed-ask](https://github.com/Trusted-Autonomy/typed-ask): a narrow, bounded-question decision primitive, `ask(question, context, schema) -> {result, confidence}`, with pluggable backends. The logic is the Rust crate's; this package is a thin PyO3 layer over it, shipped as one abi3 wheel per platform for CPython 3.10 and newer.

```python
import typed_ask

backend = typed_ask.FixtureBackend.yes_no(True, 0.93)  # deterministic, for tests
response = typed_ask.ask(
    backend,
    "Does this diff look safe to commit?",
    "3 files changed, 12 lines, tests updated",
    typed_ask.DecisionSchema.yes_no(),
)
assert response.result is True
print(response.confidence, response.model_id, response.latency_ms)
```

Any callable `fn(request: DecisionRequest) -> DecisionResponse` is a backend too:

```python
def route(request):
    label = "billing" if "charged" in request.context else "technical"
    return typed_ask.DecisionResponse(label, 0.8, model_id="rule:keyword-router")

schema = typed_ask.DecisionSchema.choice(["billing", "technical", "sales"])
typed_ask.ask(route, "Which department should handle this?", "charged twice", schema).result
# 'billing'
```

The real local-model backend, `DeciderBackend.connect(url, model_id)` / `DeciderBackend.spawn(...)`, is included in the published wheel (`typed_ask.HAS_DECIDER` is `True`). See the main README for setting up a Decider server.

## Building from source

```bash
python3 -m venv .venv && source .venv/bin/activate
pip install maturin pytest
maturin develop                  # from this directory; builds with the decider feature
pytest tests
```

Use `maturin develop` or `maturin build`, not a plain `cargo build`: maturin supplies the linker flags an extension module needs on macOS.
