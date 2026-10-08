# ta-ask (Python)

Python bindings for [ta-ask](https://github.com/Trusted-Autonomy/ta-ask): a narrow, bounded-question decision primitive, `ask(question, context, schema) -> {result, confidence}`, with pluggable backends. The logic is the Rust crate's; this package is a thin PyO3 layer over it, shipped as one abi3 wheel per platform for CPython 3.10 and newer.

```python
import ta_ask

backend = ta_ask.FixtureBackend.yes_no(True, 0.93)  # deterministic, for tests
response = ta_ask.ask(
    backend,
    "Does this diff look safe to commit?",
    "3 files changed, 12 lines, tests updated",
    ta_ask.DecisionSchema.yes_no(),
)
assert response.result is True
print(response.confidence, response.model_id, response.latency_ms)
```

Any callable `fn(request: DecisionRequest) -> DecisionResponse` is a backend too:

```python
def route(request):
    label = "billing" if "charged" in request.context else "technical"
    return ta_ask.DecisionResponse(label, 0.8, model_id="rule:keyword-router")

schema = ta_ask.DecisionSchema.choice(["billing", "technical", "sales"])
ta_ask.ask(route, "Which department should handle this?", "charged twice", schema).result
# 'billing'
```

The real local-model backend, `DeciderBackend.connect(url, model_id)` / `DeciderBackend.spawn(...)`, is included in the published wheel (`ta_ask.HAS_DECIDER` is `True`). See the main README for setting up a Decider server.

## Building from source

```bash
python3 -m venv .venv && source .venv/bin/activate
pip install maturin pytest
maturin develop                  # from this directory; builds with the decider feature
pytest tests
```

Use `maturin develop` or `maturin build`, not a plain `cargo build`: maturin supplies the linker flags an extension module needs on macOS.
