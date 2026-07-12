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

/// Point ort at the ONNX Runtime shared library shipped by the `onnxruntime`
/// Python package, unless the caller already set `ORT_DYLIB_PATH`. The extension
/// is built with ort's `load-dynamic` (it links no ONNX Runtime), so the wheel
/// stays pure and portable; onnxruntime is provided at runtime by the pip
/// package, which is broadly compatible (manylinux). This runs at import, before
/// any model loads, so ort finds the library on the first check.
fn ensure_ort_dylib(py: Python<'_>) {
    if std::env::var_os("ORT_DYLIB_PATH").is_some() {
        return;
    }
    let Ok(ort) = py.import("onnxruntime") else {
        return;
    };
    let Ok(file) = ort.getattr("__file__").and_then(|f| f.extract::<String>()) else {
        return;
    };
    let Some(capi) = std::path::Path::new(&file).parent().map(|d| d.join("capi")) else {
        return;
    };
    let mut best: Option<std::path::PathBuf> = None;
    if let Ok(entries) = std::fs::read_dir(&capi) {
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            let is_lib = name.starts_with("libonnxruntime")
                || (name.starts_with("onnxruntime") && name.ends_with(".dll"));
            if is_lib && !name.contains("providers") {
                let path = entry.path();
                // Prefer the longest filename: the real versioned library over a
                // shorter symlink or the providers shim.
                let better = best
                    .as_ref()
                    .and_then(|b| b.file_name().map(|n| n.len()))
                    .map_or(true, |cur| name.len() >= cur);
                if better {
                    best = Some(path);
                }
            }
        }
    }
    if let Some(path) = best {
        std::env::set_var("ORT_DYLIB_PATH", path);
    }
}

#[pymodule]
fn drishti(m: &Bound<'_, PyModule>) -> PyResult<()> {
    ensure_ort_dylib(m.py());
    m.add_class::<PyDrishti>()?;
    m.add("__version__", env!("CARGO_PKG_VERSION"))?;
    Ok(())
}
