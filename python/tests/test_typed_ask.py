import json
import socket
import threading
from http.server import BaseHTTPRequestHandler, HTTPServer

import pytest

import typed_ask


def test_ask_with_yes_no_fixture_backend():
    backend = typed_ask.FixtureBackend.yes_no(True, 0.9)
    response = typed_ask.ask(backend, "is this safe?", "ctx", typed_ask.DecisionSchema.yes_no())
    assert response.result is True
    assert response.confidence == 0.9
    assert response.model_id == "fixture"
    assert response.latency_ms == 0


def test_fixture_backend_with_a_choice_response():
    configured = typed_ask.DecisionResponse("billing", 0.97, model_id="fixture-choice")
    backend = typed_ask.FixtureBackend(configured)
    schema = typed_ask.DecisionSchema.choice(["billing", "technical"])
    response = typed_ask.ask(backend, "which?", "charged twice", schema)
    assert response.result == "billing"
    assert response == configured


def test_error_fixture_backend_raises_a_decision_error_subclass():
    backend = typed_ask.ErrorFixtureBackend("boom")
    with pytest.raises(typed_ask.MalformedResponseError, match="boom") as info:
        typed_ask.ask(backend, "q", "c", typed_ask.DecisionSchema.yes_no())
    assert isinstance(info.value, typed_ask.DecisionError)


def test_schema_accessors():
    yes_no = typed_ask.DecisionSchema.yes_no()
    assert (yes_no.kind, yes_no.options, yes_no.min, yes_no.max) == ("yes_no", None, None, None)
    choice = typed_ask.DecisionSchema.choice(["a", "b"])
    assert (choice.kind, choice.options) == ("choice", ["a", "b"])
    score = typed_ask.DecisionSchema.score(0.0, 10.0)
    assert (score.kind, score.min, score.max) == ("score", 0.0, 10.0)
    assert choice == typed_ask.DecisionSchema.choice(["a", "b"])
    assert "choice" in repr(choice)


def test_response_result_types_map_to_schema_kinds():
    assert typed_ask.DecisionResponse(False, 0.6, "m").result is False
    assert typed_ask.DecisionResponse("x", 0.6, "m").result == "x"
    score = typed_ask.DecisionResponse(3, 0.6, "m").result
    assert isinstance(score, float) and score == 3.0
    with pytest.raises(TypeError, match="bool .* str .* float"):
        typed_ask.DecisionResponse([1], 0.6, "m")


def test_bare_callable_is_a_backend_and_sees_the_request():
    seen = []

    def backend(request):
        seen.append(request)
        return typed_ask.DecisionResponse("technical", 0.7, model_id="py-rule", latency_ms=3)

    schema = typed_ask.DecisionSchema.choice(["billing", "technical"])
    response = typed_ask.ask(backend, "which dept?", "app crashes on start", schema)
    assert response.result == "technical"
    assert response.model_id == "py-rule"
    assert response.latency_ms == 3
    (request,) = seen
    assert request.question == "which dept?"
    assert request.context == "app crashes on start"
    assert request.schema == schema


def test_callable_backend_wrapper_is_reusable():
    backend = typed_ask.CallableBackend(lambda req: typed_ask.DecisionResponse(True, 1.0, "lambda"))
    assert isinstance(backend, typed_ask.DecisionBackend)
    for _ in range(3):
        assert typed_ask.ask(backend, "q", "c", typed_ask.DecisionSchema.yes_no()).result is True


def test_callable_backend_exception_is_reraised_unchanged():
    def backend(request):
        raise ValueError("model exploded")

    with pytest.raises(ValueError, match="model exploded"):
        typed_ask.ask(backend, "q", "c", typed_ask.DecisionSchema.yes_no())


def test_callable_backend_wrong_return_type_is_a_type_error():
    with pytest.raises(TypeError, match="must return a typed_ask.DecisionResponse, got dict"):
        typed_ask.ask(lambda req: {"result": True}, "q", "c", typed_ask.DecisionSchema.yes_no())


def test_non_backend_is_rejected():
    with pytest.raises(TypeError, match="needs a typed_ask backend or a callable"):
        typed_ask.ask(42, "q", "c", typed_ask.DecisionSchema.yes_no())
    with pytest.raises(TypeError):
        typed_ask.CallableBackend("not callable")


def test_concurrent_callable_errors_stay_on_their_own_thread():
    errors = {}

    def run(i):
        def backend(request):
            if i % 2:
                raise KeyError(f"thread-{i}")
            return typed_ask.DecisionResponse(True, 1.0, f"thread-{i}")

        try:
            errors[i] = typed_ask.ask(backend, "q", "c", typed_ask.DecisionSchema.yes_no()).model_id
        except KeyError as exc:
            errors[i] = exc.args[0]

    threads = [threading.Thread(target=run, args=(i,)) for i in range(8)]
    for t in threads:
        t.start()
    for t in threads:
        t.join()
    assert errors == {i: f"thread-{i}" for i in range(8)}


def test_module_metadata():
    assert typed_ask.__version__
    assert isinstance(typed_ask.HAS_DECIDER, bool)


decider_only = pytest.mark.skipif(not typed_ask.HAS_DECIDER, reason="built without the decider feature")


class _DeciderHandler(BaseHTTPRequestHandler):
    def do_POST(self):
        body = json.loads(self.rfile.read(int(self.headers["Content-Length"])))
        self.server.received.append(body)
        (question,) = body["schema"]
        payload = json.dumps({question: {"noul": 0.88, "type": "noul"}}).encode()
        self.send_response(200)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(payload)))
        self.end_headers()
        self.wfile.write(payload)

    def log_message(self, *args):
        pass


@decider_only
def test_decider_backend_round_trip_against_a_mock_server():
    # The mock server runs on a Python thread, so this also proves ask()
    # releases the GIL while the Rust backend blocks on HTTP.
    server = HTTPServer(("127.0.0.1", 0), _DeciderHandler)
    server.received = []
    thread = threading.Thread(target=server.serve_forever, daemon=True)
    thread.start()
    try:
        url = f"http://127.0.0.1:{server.server_address[1]}"
        backend = typed_ask.DeciderBackend.connect(url, "decider-4b-test")
        response = typed_ask.ask(backend, "Safe?", "small diff", typed_ask.DecisionSchema.yes_no())
    finally:
        server.shutdown()
    assert response.result is True
    assert response.confidence == pytest.approx(0.88)
    assert response.model_id == "decider-4b-test"
    assert server.received == [{"context": "small diff", "schema": {"Safe?": {"type": "bool"}}}]


@decider_only
def test_decider_backend_rejects_score_before_any_request():
    backend = typed_ask.DeciderBackend.connect("http://127.0.0.1:9", "decider-4b")
    with pytest.raises(typed_ask.UnsupportedSchemaError):
        typed_ask.ask(backend, "q", "c", typed_ask.DecisionSchema.score(0.0, 1.0))


@decider_only
def test_decider_backend_connection_failure_is_request_failed():
    with socket.socket() as s:
        s.bind(("127.0.0.1", 0))
        port = s.getsockname()[1]
    backend = typed_ask.DeciderBackend.connect(f"http://127.0.0.1:{port}", "decider-4b")
    with pytest.raises(typed_ask.RequestFailedError):
        typed_ask.ask(backend, "q", "c", typed_ask.DecisionSchema.yes_no())


@decider_only
def test_decider_spawn_missing_script_is_spawn_failed():
    with pytest.raises(typed_ask.SpawnFailedError, match="failed to spawn"):
        typed_ask.DeciderBackend.spawn("/definitely/not/a/real/script.sh", "m", 18240, health_timeout_secs=1)
