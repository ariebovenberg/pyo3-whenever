//! PyO3 conversions for the [whenever](https://whenever.readthedocs.io)
//! datetime library.
//!
//! Each of whenever's seven core types has a plain Rust **record** here
//! ([`Date`], [`Time`], [`PlainDateTime`], [`Instant`], [`OffsetDateTime`],
//! [`ZonedDateTime`], [`TimeDelta`]) that implements [`FromPyObject`] and
//! [`IntoPyObject`]. With the `jiff-02` feature, the records convert to and
//! from [jiff](https://docs.rs/jiff)'s types.
//!
//! The conversions go through whenever's C API when the Rust extension is
//! loaded: no Python call per value. On the pure-Python backend they fall
//! back to `to_bytes()` and `from_bytes()`, one Python call per value, with
//! the same results. The choice is made once per process, on first use.
//!
//! ```no_run
//! use pyo3::prelude::*;
//! use pyo3_whenever::Instant;
//!
//! #[pyfunction]
//! fn epoch_second(instant: Instant) -> i64 {
//!     instant.epoch_second
//! }
//! ```
//!
//! Requires whenever 1.0 or later.

mod bytes;
mod capi;
mod records;

#[cfg(feature = "jiff-02")]
mod jiff;

pub use records::{
    Date, Disambiguation, Instant, OffsetDateTime, OffsetMismatch, PlainDateTime, Time, TimeDelta,
    ZonedDateTime,
};

#[cfg(feature = "jiff-02")]
pub use jiff::Error;

use pyo3::{
    exceptions::{PyAttributeError, PyImportError},
    prelude::*,
    sync::PyOnceLock,
    types::{PyCapsule, PyType},
};

/// The seven classes, for the backend without a C API.
pub(crate) struct PyTypes {
    pub date: Py<PyType>,
    pub time: Py<PyType>,
    pub plain_datetime: Py<PyType>,
    pub instant: Py<PyType>,
    pub offset_datetime: Py<PyType>,
    pub zoned_datetime: Py<PyType>,
    pub time_delta: Py<PyType>,
}

pub(crate) enum Backend {
    /// The table, and the module whose state owns it and the type objects:
    /// held so that an unload of whenever cannot free them under the pointer.
    Capsule {
        table: &'static capi::Table,
        _module: Py<PyModule>,
    },
    Python(PyTypes),
}

// One per process, not per interpreter: whenever publishes a table per
// interpreter, but PyO3 modules refuse to load in a subinterpreter, so a
// second table is never reachable through this crate.
static BACKEND: PyOnceLock<Backend> = PyOnceLock::new();

pub(crate) fn backend(py: Python<'_>) -> PyResult<&'static Backend> {
    BACKEND.get_or_try_init(py, || select(py))
}

/// Whether conversions go through the C API, which is the case when the
/// Rust extension of whenever is loaded. Imports whenever if it has not
/// been yet.
pub fn uses_c_api(py: Python<'_>) -> PyResult<bool> {
    Ok(matches!(backend(py)?, Backend::Capsule { .. }))
}

fn select(py: Python<'_>) -> PyResult<Backend> {
    match PyCapsule::import_pointer(py, capi::CAPSULE_NAME) {
        Ok(ptr) => {
            // SAFETY: the capsule's pointer is to the table whenever's module
            // state owns, which lives as long as the module held below.
            let table = unsafe { ptr.cast::<capi::Table>().as_ref() };
            // SAFETY: `magic` is a static C string.
            let magic = unsafe { core::ffi::CStr::from_ptr(table.magic) };
            if magic != capi::CAPSULE_NAME
                || (table.struct_size as usize) < size_of::<capi::Table>()
            {
                return Err(PyImportError::new_err(
                    "whenever.C_API_v1 is not the table this version of pyo3-whenever expects",
                ));
            }
            let module = py.import("whenever._whenever")?.unbind();
            Ok(Backend::Capsule {
                table,
                _module: module,
            })
        }
        Err(e) if e.is_instance_of::<PyAttributeError>(py) => {
            let module = py.import("whenever")?;
            let class = |name: &str| -> PyResult<Py<PyType>> {
                let cls = module.getattr(name)?.cast_into::<PyType>()?;
                if !cls.hasattr("to_bytes")? {
                    return Err(PyImportError::new_err(
                        "pyo3-whenever requires whenever 1.0 or later",
                    ));
                }
                Ok(cls.unbind())
            };
            Ok(Backend::Python(PyTypes {
                date: class("Date")?,
                time: class("Time")?,
                plain_datetime: class("PlainDateTime")?,
                instant: class("Instant")?,
                offset_datetime: class("OffsetDateTime")?,
                zoned_datetime: class("ZonedDateTime")?,
                time_delta: class("TimeDelta")?,
            }))
        }
        Err(e) => Err(e),
    }
}
