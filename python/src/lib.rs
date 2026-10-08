//! Python bindings for `ta-ask`, importable as the `ta_ask` module.
//!
//! All decision logic lives in the core `ta-ask` crate; this crate only
//! converts types at the boundary. `pyo3` is a dependency of this crate
//! alone, never of the core, so `cargo test` of the core needs no Python.

use std::cell::RefCell;
use std::sync::Arc;

use pyo3::create_exception;
use pyo3::exceptions::{PyException, PyTypeError};
use pyo3::prelude::*;
use pyo3::types::{PyBool, PyFloat, PyInt, PyString};
use ta_ask as rs;

create_exception!(
    ta_ask,
    DecisionError,
    PyException,
    "Base class for every error raised by a ta_ask backend."
);
create_exception!(ta_ask, UnsupportedSchemaError, DecisionError);
create_exception!(ta_ask, SpawnFailedError, DecisionError);
create_exception!(ta_ask, BackendUnavailableError, DecisionError);
create_exception!(ta_ask, ServerExitedEarlyError, DecisionError);
create_exception!(ta_ask, RequestFailedError, DecisionError);
create_exception!(ta_ask, MalformedResponseError, DecisionError);
create_exception!(ta_ask, BackendFailedError, DecisionError);

fn to_py_err(err: rs::DecisionError) -> PyErr {
    let msg = err.to_string();
    match err {
        rs::DecisionError::UnsupportedSchema(_) => UnsupportedSchemaError::new_err(msg),
        rs::DecisionError::SpawnFailed { .. } => SpawnFailedError::new_err(msg),
        rs::DecisionError::BackendUnavailable { .. } => BackendUnavailableError::new_err(msg),
        rs::DecisionError::ServerExitedEarly { .. } => ServerExitedEarlyError::new_err(msg),
        rs::DecisionError::RequestFailed(_) => RequestFailedError::new_err(msg),
        rs::DecisionError::MalformedResponse(_) => MalformedResponseError::new_err(msg),
        rs::DecisionError::BackendFailed(_) => BackendFailedError::new_err(msg),
        _ => DecisionError::new_err(msg),
    }
}

/// The shape of answer a question expects. Build one with `yes_no()`,
/// `choice([...])` or `score(min, max)`.
#[pyclass(name = "DecisionSchema", module = "ta_ask", frozen, eq, from_py_object)]
#[derive(Clone, PartialEq)]
struct PySchema {
    inner: rs::DecisionSchema,
}

#[pymethods]
impl PySchema {
    #[staticmethod]
    fn yes_no() -> Self {
        Self {
            inner: rs::DecisionSchema::YesNo,
        }
    }

    #[staticmethod]
    fn choice(options: Vec<String>) -> Self {
        Self {
            inner: rs::DecisionSchema::Choice(options),
        }
    }

    #[staticmethod]
    fn score(min: f64, max: f64) -> Self {
        Self {
            inner: rs::DecisionSchema::Score { min, max },
        }
    }

    /// `"yes_no"`, `"choice"` or `"score"`.
    #[getter]
    fn kind(&self) -> &'static str {
        match self.inner {
            rs::DecisionSchema::YesNo => "yes_no",
            rs::DecisionSchema::Choice(_) => "choice",
            rs::DecisionSchema::Score { .. } => "score",
        }
    }

    /// The allowed options for a `choice` schema, else `None`.
    #[getter]
    fn options(&self) -> Option<Vec<String>> {
        match &self.inner {
            rs::DecisionSchema::Choice(options) => Some(options.clone()),
            _ => None,
        }
    }

    #[getter]
    fn min(&self) -> Option<f64> {
        match self.inner {
            rs::DecisionSchema::Score { min, .. } => Some(min),
            _ => None,
        }
    }

    #[getter]
    fn max(&self) -> Option<f64> {
        match self.inner {
            rs::DecisionSchema::Score { max, .. } => Some(max),
            _ => None,
        }
    }

    fn __repr__(&self) -> String {
        match &self.inner {
            rs::DecisionSchema::YesNo => "DecisionSchema.yes_no()".to_string(),
            rs::DecisionSchema::Choice(options) => format!("DecisionSchema.choice({options:?})"),
            rs::DecisionSchema::Score { min, max } => {
                format!("DecisionSchema.score({min:?}, {max:?})")
            }
        }
    }
}

/// What a backend receives: the question, its context, and the schema the
/// answer must follow.
#[pyclass(name = "DecisionRequest", module = "ta_ask", frozen, get_all)]
struct PyRequest {
    question: String,
    context: String,
    schema: PySchema,
}

#[pymethods]
impl PyRequest {
    fn __repr__(&self) -> String {
        format!(
            "DecisionRequest(question={:?}, context={:?}, schema={})",
            self.question,
            self.context,
            self.schema.__repr__()
        )
    }
}

fn result_from_py(value: &Bound<'_, PyAny>) -> PyResult<rs::DecisionResult> {
    // PyBool first: Python's bool is a subclass of int, so it would
    // otherwise be accepted as a Score.
    if value.is_instance_of::<PyBool>() {
        Ok(rs::DecisionResult::Bool(value.extract()?))
    } else if value.is_instance_of::<PyString>() {
        Ok(rs::DecisionResult::Choice(value.extract()?))
    } else if value.is_instance_of::<PyFloat>() || value.is_instance_of::<PyInt>() {
        Ok(rs::DecisionResult::Score(value.extract()?))
    } else {
        Err(PyTypeError::new_err(format!(
            "DecisionResponse.result must be a bool (yes_no), str (choice) or float (score), \
             got {}",
            value.get_type().name()?
        )))
    }
}

fn result_to_py(py: Python<'_>, result: &rs::DecisionResult) -> Py<PyAny> {
    match result {
        rs::DecisionResult::Bool(b) => PyBool::new(py, *b).to_owned().into_any().unbind(),
        rs::DecisionResult::Choice(s) => PyString::new(py, s).into_any().unbind(),
        rs::DecisionResult::Score(f) => PyFloat::new(py, *f).into_any().unbind(),
    }
}

/// A backend's answer. `result` is a `bool` for a yes/no question, the
/// chosen `str` for a choice, or a `float` for a score.
#[pyclass(
    name = "DecisionResponse",
    module = "ta_ask",
    frozen,
    eq,
    skip_from_py_object
)]
#[derive(Clone, PartialEq)]
struct PyResponse {
    inner: rs::DecisionResponse,
}

#[pymethods]
impl PyResponse {
    #[new]
    #[pyo3(signature = (result, confidence, model_id, latency_ms = 0))]
    fn new(
        result: &Bound<'_, PyAny>,
        confidence: f64,
        model_id: String,
        latency_ms: u64,
    ) -> PyResult<Self> {
        Ok(Self {
            inner: rs::DecisionResponse {
                result: result_from_py(result)?,
                confidence,
                model_id,
                latency_ms,
            },
        })
    }

    #[getter]
    fn result(&self, py: Python<'_>) -> Py<PyAny> {
        result_to_py(py, &self.inner.result)
    }

    #[getter]
    fn confidence(&self) -> f64 {
        self.inner.confidence
    }

    #[getter]
    fn model_id(&self) -> String {
        self.inner.model_id.clone()
    }

    #[getter]
    fn latency_ms(&self) -> u64 {
        self.inner.latency_ms
    }

    fn __repr__(&self) -> String {
        let result = match &self.inner.result {
            rs::DecisionResult::Bool(true) => "True".to_string(),
            rs::DecisionResult::Bool(false) => "False".to_string(),
            rs::DecisionResult::Choice(s) => format!("{s:?}"),
            rs::DecisionResult::Score(f) => format!("{f:?}"),
        };
        format!(
            "DecisionResponse(result={result}, confidence={:?}, model_id={:?}, latency_ms={})",
            self.inner.confidence, self.inner.model_id, self.inner.latency_ms
        )
    }
}

/// Base class of every backend implemented in Rust. To write a backend in
/// Python, pass a callable to `ask()` or wrap it in `CallableBackend`.
#[pyclass(name = "DecisionBackend", module = "ta_ask", subclass, frozen)]
struct PyBackend {
    inner: Arc<dyn rs::DecisionBackend>,
}

impl PyBackend {
    fn init(inner: impl rs::DecisionBackend + 'static) -> PyClassInitializer<Self> {
        PyClassInitializer::from(Self {
            inner: Arc::new(inner),
        })
    }
}

/// Always answers with the configured response. For tests.
#[pyclass(name = "FixtureBackend", module = "ta_ask", extends = PyBackend, frozen)]
struct PyFixtureBackend;

#[pymethods]
impl PyFixtureBackend {
    #[new]
    fn new(response: PyRef<'_, PyResponse>) -> PyClassInitializer<Self> {
        PyBackend::init(rs::FixtureBackend::new(response.inner.clone())).add_subclass(Self)
    }

    /// Always answers a yes/no question with `result` at `confidence`.
    #[staticmethod]
    fn yes_no(py: Python<'_>, result: bool, confidence: f64) -> PyResult<Py<Self>> {
        Py::new(
            py,
            PyBackend::init(rs::FixtureBackend::yes_no(result, confidence)).add_subclass(Self),
        )
    }
}

/// Always fails with `MalformedResponseError(message)`. For testing a
/// caller's error handling.
#[pyclass(name = "ErrorFixtureBackend", module = "ta_ask", extends = PyBackend, frozen)]
struct PyErrorFixtureBackend;

#[pymethods]
impl PyErrorFixtureBackend {
    #[new]
    fn new(message: String) -> PyClassInitializer<Self> {
        PyBackend::init(rs::ErrorFixtureBackend::new(message)).add_subclass(Self)
    }
}

thread_local! {
    /// The original Python exception raised by a callable backend during the
    /// current `ask()` on this thread, so `ask()` can re-raise it unchanged
    /// rather than as a generic `BackendFailedError`. The backend runs on
    /// the same thread as the `ask()` that called it, so a thread-local is
    /// exact even when several Python threads call `ask()` concurrently.
    static PENDING_PY_ERR: RefCell<Option<PyErr>> = const { RefCell::new(None) };
}

/// A Rust `DecisionBackend` that calls a Python callable
/// `fn(request: DecisionRequest) -> DecisionResponse`.
struct CallableDecisionBackend {
    callable: Py<PyAny>,
}

impl CallableDecisionBackend {
    fn call(&self, py: Python<'_>, req: &rs::DecisionRequest) -> PyResult<rs::DecisionResponse> {
        let request = PyRequest {
            question: req.question.clone(),
            context: req.context.clone(),
            schema: PySchema {
                inner: req.schema.clone(),
            },
        };
        let returned = self.callable.call1(py, (request,))?;
        let returned = returned.bind(py);
        match returned.cast::<PyResponse>() {
            Ok(response) => Ok(response.get().inner.clone()),
            Err(_) => Err(PyTypeError::new_err(format!(
                "a callable backend must return a ta_ask.DecisionResponse, got {}",
                returned.get_type().name()?
            ))),
        }
    }
}

impl rs::DecisionBackend for CallableDecisionBackend {
    fn decide(&self, req: &rs::DecisionRequest) -> Result<rs::DecisionResponse, rs::DecisionError> {
        Python::attach(|py| {
            self.call(py, req).map_err(|err| {
                let msg = format!("Python callable backend raised: {err}");
                PENDING_PY_ERR.with(|slot| *slot.borrow_mut() = Some(err));
                rs::DecisionError::BackendFailed(msg)
            })
        })
    }
}

/// Wraps a Python callable `fn(request: DecisionRequest) -> DecisionResponse`
/// as a backend. `ask()` also accepts the bare callable directly.
#[pyclass(name = "CallableBackend", module = "ta_ask", extends = PyBackend, frozen)]
struct PyCallableBackend;

#[pymethods]
impl PyCallableBackend {
    #[new]
    fn new(callable: &Bound<'_, PyAny>) -> PyResult<PyClassInitializer<Self>> {
        if !callable.is_callable() {
            return Err(PyTypeError::new_err(
                "CallableBackend needs a callable taking a DecisionRequest and returning a \
                 DecisionResponse",
            ));
        }
        Ok(PyBackend::init(CallableDecisionBackend {
            callable: callable.clone().unbind(),
        })
        .add_subclass(Self))
    }
}

#[cfg(feature = "decider")]
mod decider {
    use std::collections::HashMap;
    use std::path::PathBuf;
    use std::time::Duration;

    use pyo3::exceptions::PyValueError;

    use super::*;

    /// The real local-model backend: talks HTTP to a Decider model server,
    /// optionally spawning it. Construct with `connect()` or `spawn()`.
    #[pyclass(name = "DeciderBackend", module = "ta_ask", extends = PyBackend, frozen)]
    pub(super) struct PyDeciderBackend;

    #[pymethods]
    impl PyDeciderBackend {
        /// Use an already-running Decider server at `base_url`. `model_id` is
        /// only a label copied into each response.
        #[staticmethod]
        fn connect(py: Python<'_>, base_url: String, model_id: String) -> PyResult<Py<Self>> {
            let backend = py.detach(|| rs::DeciderBackend::connect(base_url, model_id));
            Py::new(py, PyBackend::init(backend).add_subclass(Self))
        }

        /// Run `serve_script model_id port` (Decider's `scripts/serve.sh`),
        /// wait up to `health_timeout_secs` for it to become healthy, and
        /// own the process: it is killed when this backend is garbage
        /// collected. `model_id` must be the full Hugging Face repo id.
        #[staticmethod]
        #[pyo3(signature = (serve_script, model_id, port, env = None, health_timeout_secs = 60.0))]
        fn spawn(
            py: Python<'_>,
            serve_script: PathBuf,
            model_id: String,
            port: u16,
            env: Option<HashMap<String, String>>,
            health_timeout_secs: f64,
        ) -> PyResult<Py<Self>> {
            let env = env.unwrap_or_default();
            let timeout = Duration::try_from_secs_f64(health_timeout_secs).map_err(|e| {
                PyValueError::new_err(format!(
                    "health_timeout_secs must be a non-negative, finite number of seconds: {e}"
                ))
            })?;
            let backend = py
                .detach(|| {
                    let env: Vec<(&str, &str)> =
                        env.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
                    rs::DeciderBackend::spawn(&serve_script, &model_id, port, &env, timeout)
                })
                .map_err(to_py_err)?;
            Py::new(py, PyBackend::init(backend).add_subclass(Self))
        }
    }
}

/// Ask `backend` one bounded question about `context` and return its
/// answer. `backend` is any `ta_ask` backend, or a Python callable
/// `fn(request: DecisionRequest) -> DecisionResponse`. The GIL is released
/// while a Rust backend works, so other Python threads keep running.
#[pyfunction]
fn ask(
    py: Python<'_>,
    backend: &Bound<'_, PyAny>,
    question: String,
    context: String,
    schema: PySchema,
) -> PyResult<PyResponse> {
    let inner: Arc<dyn rs::DecisionBackend> = if let Ok(b) = backend.cast::<PyBackend>() {
        Arc::clone(&b.get().inner)
    } else if backend.is_callable() {
        Arc::new(CallableDecisionBackend {
            callable: backend.clone().unbind(),
        })
    } else {
        return Err(PyTypeError::new_err(format!(
            "ask() needs a ta_ask backend or a callable taking a DecisionRequest, got {}",
            backend.get_type().name()?
        )));
    };

    PENDING_PY_ERR.with(|slot| slot.borrow_mut().take());
    let outcome = py.detach(|| rs::ask(inner.as_ref(), question, context, schema.inner));
    match outcome {
        Ok(inner) => Ok(PyResponse { inner }),
        Err(err) => match PENDING_PY_ERR.with(|slot| slot.borrow_mut().take()) {
            Some(original) => Err(original),
            None => Err(to_py_err(err)),
        },
    }
}

#[pymodule]
#[pyo3(name = "ta_ask")]
fn ta_ask_module(m: &Bound<'_, PyModule>) -> PyResult<()> {
    let py = m.py();
    m.add("__version__", env!("CARGO_PKG_VERSION"))?;
    m.add("HAS_DECIDER", cfg!(feature = "decider"))?;
    m.add_function(wrap_pyfunction!(ask, m)?)?;
    m.add_class::<PySchema>()?;
    m.add_class::<PyRequest>()?;
    m.add_class::<PyResponse>()?;
    m.add_class::<PyBackend>()?;
    m.add_class::<PyFixtureBackend>()?;
    m.add_class::<PyErrorFixtureBackend>()?;
    m.add_class::<PyCallableBackend>()?;
    #[cfg(feature = "decider")]
    m.add_class::<decider::PyDeciderBackend>()?;
    m.add("DecisionError", py.get_type::<DecisionError>())?;
    m.add(
        "UnsupportedSchemaError",
        py.get_type::<UnsupportedSchemaError>(),
    )?;
    m.add("SpawnFailedError", py.get_type::<SpawnFailedError>())?;
    m.add(
        "BackendUnavailableError",
        py.get_type::<BackendUnavailableError>(),
    )?;
    m.add(
        "ServerExitedEarlyError",
        py.get_type::<ServerExitedEarlyError>(),
    )?;
    m.add("RequestFailedError", py.get_type::<RequestFailedError>())?;
    m.add(
        "MalformedResponseError",
        py.get_type::<MalformedResponseError>(),
    )?;
    m.add("BackendFailedError", py.get_type::<BackendFailedError>())?;
    Ok(())
}
