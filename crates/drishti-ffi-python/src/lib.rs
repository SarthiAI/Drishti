//! Python bindings for Drishti. A thin adapter over `drishti-core`: each method
//! calls the same core function the CLI and server call, so results match across
//! every surface (invariant I5). Methods release the GIL during inference and
//! return plain Python objects (dicts) shaped from the core result types.
//!
//! v0.1 exposes blocking methods, which is the correct interface for CPU-bound
//! inference. Async (`async def`) variants over `pyo3-async-runtimes` are
//! recorded as backlog; they add a thread offload but no real concurrency for
//! CPU work.

use std::sync::Arc;

use drishti_core::config::DrishtiConfig;
use drishti_core::Drishti as Core;
use drishti_models::FsSource;
use pyo3::exceptions::{PyRuntimeError, PyValueError};
use pyo3::prelude::*;
use pyo3::types::PyAny;
use serde::Serialize;

/// A built Drishti instance. Construct with [`Drishti.from_config_file`] or
/// [`Drishti.from_config_str`].
#[pyclass(name = "Drishti")]
struct PyDrishti {
    inner: Arc<Core>,
}

#[pymethods]
impl PyDrishti {
    /// Build from a TOML config file path. Resolves and loads every enabled
    /// check's model (downloading if not cached).
    #[staticmethod]
    fn from_config_file(path: String) -> PyResult<Self> {
        let text = std::fs::read_to_string(&path)
            .map_err(|e| PyValueError::new_err(format!("read config {path}: {e}")))?;
        Self::build(text)
    }

    /// Build from a TOML config string.
    #[staticmethod]
    fn from_config_str(toml_text: String) -> PyResult<Self> {
        Self::build(toml_text)
    }

    /// Prompt-injection check. Returns a dict.
    fn check_prompt<'py>(&self, py: Python<'py>, text: String) -> PyResult<Bound<'py, PyAny>> {
        let inner = self.inner.clone();
        let result = py
            .detach(|| futures::executor::block_on(inner.check_prompt(&text)))
            .map_err(to_pyerr)?;
        to_py(py, &result)
    }

    /// PII detection and redaction. Returns a dict.
    fn check_pii<'py>(&self, py: Python<'py>, text: String) -> PyResult<Bound<'py, PyAny>> {
        let inner = self.inner.clone();
        let result = py
            .detach(|| futures::executor::block_on(inner.check_pii(&text)))
            .map_err(to_pyerr)?;
        to_py(py, &result)
    }

    /// Output-safety check. Returns a dict.
    fn check_output<'py>(&self, py: Python<'py>, text: String) -> PyResult<Bound<'py, PyAny>> {
        let inner = self.inner.clone();
        let result = py
            .detach(|| futures::executor::block_on(inner.check_output(&text)))
            .map_err(to_pyerr)?;
        to_py(py, &result)
    }

    /// Run every enabled check. The text is the prompt; pass `output` to also run
    /// the output-safety check on a separate string. Returns a dict.
    #[pyo3(signature = (prompt, output=None))]
    fn check_all<'py>(
        &self,
        py: Python<'py>,
        prompt: String,
        output: Option<String>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let inner = self.inner.clone();
        let result = py
            .detach(|| {
                futures::executor::block_on(inner.check_all(&prompt, output.as_deref()))
            })
            .map_err(to_pyerr)?;
        to_py(py, &result)
    }

    /// The loaded model manifest (ids and hashes), for audit. Returns a dict.
    fn manifest<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        to_py(py, &self.inner.model_manifest())
    }
}

impl PyDrishti {
    fn build(toml_text: String) -> PyResult<Self> {
        dotenvy::dotenv().ok();
        let config = DrishtiConfig::from_toml_and_env(&toml_text).map_err(to_pyerr)?;
        let source = FsSource::with_optional_cache(config.cache_dir.clone());
        let inner = Core::builder()
            .with_config(config)
            .build(&source)
            .map_err(to_pyerr)?;
        Ok(Self {
            inner: Arc::new(inner),
        })
    }
}

fn to_pyerr<E: std::fmt::Display>(e: E) -> PyErr {
    PyRuntimeError::new_err(e.to_string())
}

fn to_py<'py, T: Serialize>(py: Python<'py>, value: &T) -> PyResult<Bound<'py, PyAny>> {
    pythonize::pythonize(py, value).map_err(to_pyerr)
}

#[pymodule]
fn drishti(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<PyDrishti>()?;
    m.add("__version__", env!("CARGO_PKG_VERSION"))?;
    Ok(())
}
