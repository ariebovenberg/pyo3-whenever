//! The records: whenever's classes as plain Rust structs, with their
//! conversions to and from Python objects.
//!
//! A record has the fields that whenever stores, and nothing checks them
//! until whenever builds an object from the record. Extraction accepts
//! exactly the class of the record.
//!
//! ```no_run
//! use pyo3::prelude::*;
//! use pyo3_whenever::records::Instant;
//!
//! #[pyfunction]
//! fn epoch_second(instant: Instant) -> i64 {
//!     instant.epoch_second
//! }
//! ```
//!
//! With the `jiff-02` feature, the records convert to and from jiff's
//! types through `TryFrom`.

use crate::{Backend, backend, bytes, capi};
use core::{ffi::c_int, mem::MaybeUninit};
use pyo3::{
    exceptions::{PyTypeError, PyValueError},
    intern,
    prelude::*,
    sync::PyOnceLock,
    types::{PyBytes, PyDict, PyTuple, PyType},
};

/// A calendar date in the proleptic Gregorian calendar, year 1 to 9999.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Date {
    pub year: u16,
    pub month: u8,
    pub day: u8,
}

/// A time of day with nanosecond precision.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Time {
    pub hour: u8,
    pub minute: u8,
    pub second: u8,
    pub nanosecond: u32,
}

/// A date and time without a time zone or offset.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct PlainDateTime {
    pub date: Date,
    pub time: Time,
}

/// A moment on the timeline, as seconds since the Unix epoch and a
/// nanosecond within that second. Year 1 to 9999 in UTC.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Instant {
    pub epoch_second: i64,
    pub nanosecond: u32,
}

/// A local date and time with the UTC offset it was observed at, in
/// seconds, within ±23:59:59.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct OffsetDateTime {
    pub local: PlainDateTime,
    pub offset_second: i32,
}

/// An [`OffsetDateTime`] in a named time zone: the local fields and the
/// offset as whenever stored them, plus the time zone ID. The ID is `None`
/// for a system time zone without one; such a value converts to Rust but
/// not back.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct ZonedDateTime {
    pub odt: OffsetDateTime,
    pub tz_id: Option<String>,
}

/// A duration as whole seconds and a never-negative nanosecond: the total
/// is `second * 10⁹ + nanosecond`, so one nanosecond before zero is
/// `second: -1, nanosecond: 999_999_999`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct TimeDelta {
    pub second: i64,
    pub nanosecond: u32,
}

/// An `ItemizedDelta`: each component, or `None` when it is absent. A
/// present zero is `Some(0)`, so `==` tells them apart, as `strict_eq()`
/// does in Python. The empty record, [`Default`], is the empty delta.
///
/// whenever requires one sign for all components, a `nanoseconds` within
/// ±999,999,999, and limits per unit.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct ItemizedDelta {
    pub years: Option<i32>,
    pub months: Option<i32>,
    pub weeks: Option<i32>,
    pub days: Option<i32>,
    pub hours: Option<i32>,
    pub minutes: Option<i64>,
    pub seconds: Option<i64>,
    pub nanoseconds: Option<i32>,
}

/// An `ItemizedDateDelta`: the date components of an [`ItemizedDelta`],
/// with the same rules.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct ItemizedDateDelta {
    pub years: Option<i32>,
    pub months: Option<i32>,
    pub weeks: Option<i32>,
    pub days: Option<i32>,
}

/// What a `ZonedDateTime` does when its offset identifies no occurrence of
/// its local time in the time zone: `offset_mismatch=` in Python.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OffsetMismatch {
    Raise = 0,
    KeepInstant = 1,
    KeepLocal = 2,
}

impl OffsetMismatch {
    fn as_str(self) -> &'static str {
        match self {
            Self::Raise => "raise",
            Self::KeepInstant => "keep_instant",
            Self::KeepLocal => "keep_local",
        }
    }
}

/// Which occurrence a repeated local time means, and what a skipped one
/// becomes: `disambiguation=` in Python. Consulted under
/// [`OffsetMismatch::KeepLocal`] only.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Disambiguation {
    Compatible = 0,
    Earlier = 1,
    Later = 2,
    Raise = 3,
}

impl Disambiguation {
    fn as_str(self) -> &'static str {
        match self {
            Self::Compatible => "compatible",
            Self::Earlier => "earlier",
            Self::Later => "later",
            Self::Raise => "raise",
        }
    }
}

/// Days from 1970-01-01 to the given date (Howard Hinnant's algorithm).
fn days_from_civil(d: Date) -> i64 {
    let y = i64::from(d.year) - i64::from(d.month <= 2);
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let m = i64::from(d.month);
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + i64::from(d.day) - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

impl PlainDateTime {
    /// Seconds since the Unix epoch if this local time were UTC.
    fn epoch_second_as_utc(self) -> i64 {
        days_from_civil(self.date) * 86_400
            + i64::from(self.time.hour) * 3_600
            + i64::from(self.time.minute) * 60
            + i64::from(self.time.second)
    }
}

impl Instant {
    /// A whenever `ZonedDateTime` of this instant in the named time zone:
    /// `Instant.to_tz()` in Python. whenever's time zone data computes the
    /// local fields. Raises `TimeZoneNotFoundError` for an unknown ID.
    pub fn to_tz<'py>(self, py: Python<'py>, tz_id: &str) -> PyResult<Bound<'py, PyAny>> {
        match backend(py)? {
            Backend::Capsule { table: t, .. } => {
                let tz_id_len = u8::try_from(tz_id.len())
                    .map_err(|_| PyValueError::new_err("time zone ID too long"))?;
                let rec = capi::Instant::from(self);
                // SAFETY: the table is the capsule's; the pointers are valid for the call.
                let ptr = unsafe {
                    (t.zoned_datetime_from_instant)(*t, &rec, tz_id.as_ptr().cast(), tz_id_len)
                };
                unsafe { Bound::from_owned_ptr_or_err(py, ptr) }
            }
            Backend::Python(types) => types
                .instant
                .bind(py)
                .call_method1(
                    "from_bytes",
                    (PyBytes::new(py, &bytes::encode_instant(self)),),
                )?
                .call_method1("to_tz", (tz_id,)),
        }
    }
}

impl OffsetDateTime {
    /// The moment this value denotes.
    pub fn instant(self) -> Instant {
        Instant {
            epoch_second: self.local.epoch_second_as_utc() - i64::from(self.offset_second),
            nanosecond: self.local.time.nanosecond,
        }
    }
}

impl ZonedDateTime {
    /// The moment this value denotes, from its local fields and offset.
    pub fn instant(&self) -> Instant {
        self.odt.instant()
    }

    /// The record as written, resolved against whenever's time zone data
    /// under the given policies. [`IntoPyObject`] uses
    /// [`OffsetMismatch::Raise`], which never consults the disambiguation.
    pub fn to_pyobject_with<'py>(
        &self,
        py: Python<'py>,
        offset_mismatch: OffsetMismatch,
        disambiguation: Disambiguation,
    ) -> PyResult<Bound<'py, PyAny>> {
        let tz_id = self
            .tz_id
            .as_deref()
            .ok_or_else(|| PyValueError::new_err("ZonedDateTime without a time zone ID"))?;
        match backend(py)? {
            Backend::Capsule { table: t, .. } => {
                let tz_id_len = u8::try_from(tz_id.len())
                    .map_err(|_| PyValueError::new_err("time zone ID too long"))?;
                let rec = capi::ZonedDateTime {
                    odt: self.odt.into(),
                    tz_id: tz_id.as_ptr().cast(),
                    tz_id_len,
                };
                // SAFETY: the table is the capsule's; `rec` borrows `tz_id` for the call.
                let ptr = unsafe {
                    (t.zoned_datetime_new)(
                        *t,
                        &rec,
                        offset_mismatch as c_int,
                        disambiguation as c_int,
                    )
                };
                unsafe { Bound::from_owned_ptr_or_err(py, ptr) }
            }
            Backend::Python(types) => {
                let kwargs = PyDict::new(py);
                kwargs.set_item("offset_mismatch", offset_mismatch.as_str())?;
                kwargs.set_item("disambiguation", disambiguation.as_str())?;
                types.zoned_datetime.bind(py).call_method(
                    "from_bytes",
                    (PyBytes::new(py, &bytes::encode_zoned(self, tz_id)),),
                    Some(&kwargs),
                )
            }
        }
    }
}

// --- records <-> C records -------------------------------------------------

impl From<capi::Date> for Date {
    fn from(r: capi::Date) -> Self {
        Self {
            year: r.year,
            month: r.month,
            day: r.day,
        }
    }
}

impl From<Date> for capi::Date {
    fn from(d: Date) -> Self {
        Self {
            year: d.year,
            month: d.month,
            day: d.day,
        }
    }
}

impl From<capi::Time> for Time {
    fn from(r: capi::Time) -> Self {
        Self {
            hour: r.hour,
            minute: r.minute,
            second: r.second,
            nanosecond: r.nanosecond,
        }
    }
}

impl From<Time> for capi::Time {
    fn from(t: Time) -> Self {
        Self {
            hour: t.hour,
            minute: t.minute,
            second: t.second,
            nanosecond: t.nanosecond,
        }
    }
}

impl From<capi::PlainDateTime> for PlainDateTime {
    fn from(r: capi::PlainDateTime) -> Self {
        Self {
            date: r.date.into(),
            time: r.time.into(),
        }
    }
}

impl From<PlainDateTime> for capi::PlainDateTime {
    fn from(p: PlainDateTime) -> Self {
        Self {
            date: p.date.into(),
            time: p.time.into(),
        }
    }
}

impl From<capi::Instant> for Instant {
    fn from(r: capi::Instant) -> Self {
        Self {
            epoch_second: r.epoch_second,
            nanosecond: r.nanosecond,
        }
    }
}

impl From<Instant> for capi::Instant {
    fn from(i: Instant) -> Self {
        Self {
            epoch_second: i.epoch_second,
            nanosecond: i.nanosecond,
        }
    }
}

impl From<capi::OffsetDateTime> for OffsetDateTime {
    fn from(r: capi::OffsetDateTime) -> Self {
        Self {
            local: r.local.into(),
            offset_second: r.offset_second,
        }
    }
}

impl From<OffsetDateTime> for capi::OffsetDateTime {
    fn from(o: OffsetDateTime) -> Self {
        Self {
            local: o.local.into(),
            offset_second: o.offset_second,
        }
    }
}

impl From<capi::TimeDelta> for TimeDelta {
    fn from(r: capi::TimeDelta) -> Self {
        Self {
            second: r.second,
            nanosecond: r.nanosecond,
        }
    }
}

impl From<TimeDelta> for capi::TimeDelta {
    fn from(d: TimeDelta) -> Self {
        Self {
            second: d.second,
            nanosecond: d.nanosecond,
        }
    }
}

// --- Python conversions ------------------------------------------------------

/// The error for an object of another class. `expected` names the classes
/// that are accepted.
pub(crate) fn wrong_type(ob: &Bound<'_, PyAny>, expected: &str) -> PyErr {
    PyTypeError::new_err(format!(
        "expected a {expected}, got {}",
        ob.get_type()
            .name()
            .map_or_else(|_| "?".to_owned(), |n| n.to_string()),
    ))
}

/// The record from `to_bytes()`, for the backend without a C API.
fn from_to_bytes<T>(ob: &Bound<'_, PyAny>, decode: impl Fn(&[u8]) -> PyResult<T>) -> PyResult<T> {
    let data = ob.call_method0("to_bytes")?;
    decode(data.extract::<&[u8]>()?)
}

/// Extract through the capsule, or through `fallback` on the backend
/// without one. `None` when the object is not exactly of the class.
fn try_extract<'py, T, R: Copy>(
    ob: &Bound<'py, PyAny>,
    slot: impl Fn(&capi::Table) -> capi::Extract<R>,
    py_type: impl Fn(&crate::PyTypes) -> &Py<pyo3::types::PyType>,
    fallback: impl Fn(&Bound<'py, PyAny>) -> PyResult<T>,
) -> PyResult<Option<T>>
where
    T: From<R>,
{
    match backend(ob.py())? {
        Backend::Capsule { table: t, .. } => {
            let mut rec = MaybeUninit::<R>::uninit();
            // SAFETY: the table is the capsule's; `ob` is a live object; `rec` is ours to fill.
            match unsafe { slot(t)(*t, ob.as_ptr(), rec.as_mut_ptr()) } {
                // SAFETY: 0 means the extractor filled the record.
                0 => Ok(Some(unsafe { rec.assume_init() }.into())),
                _ => Ok(None),
            }
        }
        Backend::Python(types) => ob
            .get_type()
            .is(py_type(types))
            .then(|| fallback(ob))
            .transpose(),
    }
}

/// Construct through the capsule, or through `from_bytes()` on the backend
/// without one.
fn construct<'py, R>(
    py: Python<'py>,
    rec: R,
    slot: impl Fn(&capi::Table) -> capi::New<R>,
    py_type: impl Fn(&crate::PyTypes) -> &Py<pyo3::types::PyType>,
    encoded: &[u8],
) -> PyResult<Bound<'py, PyAny>> {
    match backend(py)? {
        Backend::Capsule { table: t, .. } => {
            // SAFETY: the table is the capsule's; `rec` is valid for the call.
            let ptr = unsafe { slot(t)(*t, &rec) };
            unsafe { Bound::from_owned_ptr_or_err(py, ptr) }
        }
        Backend::Python(types) => py_type(types)
            .bind(py)
            .call_method1("from_bytes", (PyBytes::new(py, encoded),)),
    }
}

macro_rules! conversions {
    ($typ:ident, $name:literal, $extract:ident, $new:ident, $slot:ident, $decode:path, $encode:path) => {
        impl $typ {
            /// The record, or `None` when the object is not exactly of
            /// the class.
            pub(crate) fn try_extract(ob: &Bound<'_, PyAny>) -> PyResult<Option<Self>> {
                try_extract(
                    ob,
                    |t| t.$extract,
                    |types| &types.$slot,
                    |ob| from_to_bytes(ob, $decode),
                )
            }
        }

        impl<'py> FromPyObject<'_, 'py> for $typ {
            type Error = PyErr;

            fn extract(ob: Borrowed<'_, 'py, PyAny>) -> PyResult<Self> {
                Self::try_extract(&ob)?.ok_or_else(|| wrong_type(&ob, $name))
            }
        }

        impl<'py> IntoPyObject<'py> for $typ {
            type Target = PyAny;
            type Output = Bound<'py, PyAny>;
            type Error = PyErr;

            fn into_pyobject(self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
                construct(
                    py,
                    capi::$typ::from(self),
                    |t| t.$new,
                    |types| &types.$slot,
                    &$encode(self),
                )
            }
        }

        impl<'py> IntoPyObject<'py> for &$typ {
            type Target = PyAny;
            type Output = Bound<'py, PyAny>;
            type Error = PyErr;

            fn into_pyobject(self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
                (*self).into_pyobject(py)
            }
        }
    };
}

conversions!(
    Date,
    "whenever.Date",
    date_extract,
    date_new,
    date,
    bytes::decode_date,
    bytes::encode_date
);
conversions!(
    Time,
    "whenever.Time",
    time_extract,
    time_new,
    time,
    bytes::decode_time,
    bytes::encode_time
);
conversions!(
    PlainDateTime,
    "whenever.PlainDateTime",
    plain_datetime_extract,
    plain_datetime_new,
    plain_datetime,
    bytes::decode_plain,
    bytes::encode_plain
);
conversions!(
    Instant,
    "whenever.Instant",
    instant_extract,
    instant_new,
    instant,
    bytes::decode_instant,
    bytes::encode_instant
);
conversions!(
    OffsetDateTime,
    "whenever.OffsetDateTime",
    offset_datetime_extract,
    offset_datetime_new,
    offset_datetime,
    bytes::decode_offset,
    bytes::encode_offset
);
conversions!(
    TimeDelta,
    "whenever.TimeDelta",
    time_delta_extract,
    time_delta_new,
    time_delta,
    bytes::decode_time_delta,
    bytes::encode_time_delta
);

impl ZonedDateTime {
    /// Calls `f` with the local fields, the offset, and the time zone ID,
    /// borrowed from the object or from its `to_bytes()`. `None` when the
    /// object is not exactly a `ZonedDateTime`.
    pub(crate) fn try_extract_with<R>(
        ob: &Bound<'_, PyAny>,
        f: impl FnOnce(OffsetDateTime, Option<&str>) -> R,
    ) -> PyResult<Option<R>> {
        match backend(ob.py())? {
            Backend::Capsule { table: t, .. } => {
                let mut rec = MaybeUninit::<capi::ZonedDateTime>::uninit();
                // SAFETY: the table is the capsule's; `ob` is a live object; `rec` is ours to fill.
                if unsafe { (t.zoned_datetime_extract)(*t, ob.as_ptr(), rec.as_mut_ptr()) } != 0 {
                    return Ok(None);
                }
                // SAFETY: 0 means the extractor filled the record.
                let r = unsafe { rec.assume_init() };
                let tz_id = (!r.tz_id.is_null())
                    .then(|| {
                        // SAFETY: a non-null ID is `tz_id_len` bytes owned by
                        // `ob`, which outlives this call.
                        let b = unsafe {
                            core::slice::from_raw_parts(
                                r.tz_id.cast::<u8>(),
                                usize::from(r.tz_id_len),
                            )
                        };
                        str::from_utf8(b).map_err(|_| {
                            PyValueError::new_err(
                                "whenever.ZonedDateTime with a non-UTF-8 time zone ID",
                            )
                        })
                    })
                    .transpose()?;
                Ok(Some(f(r.odt.into(), tz_id)))
            }
            Backend::Python(types) => {
                if !ob.get_type().is(&types.zoned_datetime) {
                    return Ok(None);
                }
                // `to_bytes()` refuses a system time zone without an ID,
                // which the capsule extracts with a null ID.
                if ob.getattr("tz_id")?.is_none() {
                    let odt = ob.call_method0("to_fixed_offset")?;
                    return Ok(Some(f(from_to_bytes(&odt, bytes::decode_offset)?, None)));
                }
                let data = ob.call_method0("to_bytes")?;
                let (odt, tz_id) = bytes::decode_zoned(data.extract::<&[u8]>()?)?;
                Ok(Some(f(odt, Some(tz_id))))
            }
        }
    }

    /// The record, or `None` when the object is not exactly a
    /// `ZonedDateTime`. Copies the time zone ID.
    pub(crate) fn try_extract(ob: &Bound<'_, PyAny>) -> PyResult<Option<Self>> {
        Self::try_extract_with(ob, |odt, tz_id| Self {
            odt,
            tz_id: tz_id.map(str::to_owned),
        })
    }
}

impl<'py> FromPyObject<'_, 'py> for ZonedDateTime {
    type Error = PyErr;

    fn extract(ob: Borrowed<'_, 'py, PyAny>) -> PyResult<Self> {
        Self::try_extract(&ob)?.ok_or_else(|| wrong_type(&ob, "whenever.ZonedDateTime"))
    }
}

impl<'py> IntoPyObject<'py> for &ZonedDateTime {
    type Target = PyAny;
    type Output = Bound<'py, PyAny>;
    type Error = PyErr;

    fn into_pyobject(self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        self.to_pyobject_with(py, OffsetMismatch::Raise, Disambiguation::Compatible)
    }
}

impl<'py> IntoPyObject<'py> for ZonedDateTime {
    type Target = PyAny;
    type Output = Bound<'py, PyAny>;
    type Error = PyErr;

    fn into_pyobject(self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        (&self).into_pyobject(py)
    }
}

// The itemized deltas are Python classes on both backends, so they have no
// C API. They convert as pickle does: the record has the arguments that
// `__reduce__()` gives and the unpickle function takes.

/// The itemized delta classes and their unpickle functions.
struct Itemized {
    delta: Py<PyType>,
    date_delta: Py<PyType>,
    unpickle_delta: Py<PyAny>,
    unpickle_date_delta: Py<PyAny>,
}

static ITEMIZED: PyOnceLock<Itemized> = PyOnceLock::new();

fn itemized(py: Python<'_>) -> PyResult<&'static Itemized> {
    ITEMIZED.get_or_try_init(py, || {
        let module = py.import("whenever")?;
        let class = |name: &str| -> PyResult<Py<PyType>> {
            Ok(module.getattr(name)?.cast_into::<PyType>()?.unbind())
        };
        Ok(Itemized {
            delta: class("ItemizedDelta")?,
            date_delta: class("ItemizedDateDelta")?,
            unpickle_delta: module.getattr("_unpkl_idelta")?.unbind(),
            unpickle_date_delta: module.getattr("_unpkl_iddelta")?.unbind(),
        })
    })
}

/// The arguments of the pickle payload, or `None` when the object is not
/// exactly of `class`.
fn pickle_args<'py>(
    ob: &Bound<'py, PyAny>,
    class: &Py<PyType>,
    unpickle: &Py<PyAny>,
) -> PyResult<Option<Bound<'py, PyTuple>>> {
    if !ob.get_type().is(class) {
        return Ok(None);
    }
    let (f, args): (Bound<'py, PyAny>, Bound<'py, PyTuple>) =
        ob.call_method0(intern!(ob.py(), "__reduce__"))?.extract()?;
    if !f.is(unpickle) {
        return Err(PyValueError::new_err(
            "whenever pickles itemized deltas in a format this version of pyo3-whenever does not read",
        ));
    }
    Ok(Some(args))
}

impl ItemizedDelta {
    /// The record, or `None` when the object is not exactly an
    /// `ItemizedDelta`.
    pub(crate) fn try_extract(ob: &Bound<'_, PyAny>) -> PyResult<Option<Self>> {
        let i = itemized(ob.py())?;
        pickle_args(ob, &i.delta, &i.unpickle_delta)?
            .map(|a| {
                let (years, months, weeks, days, hours, minutes, seconds, nanoseconds) =
                    a.extract()?;
                Ok(Self {
                    years,
                    months,
                    weeks,
                    days,
                    hours,
                    minutes,
                    seconds,
                    nanoseconds,
                })
            })
            .transpose()
    }
}

impl ItemizedDateDelta {
    /// The record, or `None` when the object is not exactly an
    /// `ItemizedDateDelta`.
    pub(crate) fn try_extract(ob: &Bound<'_, PyAny>) -> PyResult<Option<Self>> {
        let i = itemized(ob.py())?;
        pickle_args(ob, &i.date_delta, &i.unpickle_date_delta)?
            .map(|a| {
                let (years, months, weeks, days) = a.extract()?;
                Ok(Self {
                    years,
                    months,
                    weeks,
                    days,
                })
            })
            .transpose()
    }
}

impl<'py> FromPyObject<'_, 'py> for ItemizedDelta {
    type Error = PyErr;

    fn extract(ob: Borrowed<'_, 'py, PyAny>) -> PyResult<Self> {
        Self::try_extract(&ob)?.ok_or_else(|| wrong_type(&ob, "whenever.ItemizedDelta"))
    }
}

impl<'py> FromPyObject<'_, 'py> for ItemizedDateDelta {
    type Error = PyErr;

    fn extract(ob: Borrowed<'_, 'py, PyAny>) -> PyResult<Self> {
        Self::try_extract(&ob)?.ok_or_else(|| wrong_type(&ob, "whenever.ItemizedDateDelta"))
    }
}

impl<'py> IntoPyObject<'py> for ItemizedDelta {
    type Target = PyAny;
    type Output = Bound<'py, PyAny>;
    type Error = PyErr;

    fn into_pyobject(self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let Self {
            years,
            months,
            weeks,
            days,
            hours,
            minutes,
            seconds,
            nanoseconds,
        } = self;
        itemized(py)?.unpickle_delta.bind(py).call1((
            years,
            months,
            weeks,
            days,
            hours,
            minutes,
            seconds,
            nanoseconds,
        ))
    }
}

impl<'py> IntoPyObject<'py> for &ItemizedDelta {
    type Target = PyAny;
    type Output = Bound<'py, PyAny>;
    type Error = PyErr;

    fn into_pyobject(self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        (*self).into_pyobject(py)
    }
}

impl<'py> IntoPyObject<'py> for ItemizedDateDelta {
    type Target = PyAny;
    type Output = Bound<'py, PyAny>;
    type Error = PyErr;

    fn into_pyobject(self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let Self {
            years,
            months,
            weeks,
            days,
        } = self;
        itemized(py)?
            .unpickle_date_delta
            .bind(py)
            .call1((years, months, weeks, days))
    }
}

impl<'py> IntoPyObject<'py> for &ItemizedDateDelta {
    type Target = PyAny;
    type Output = Bound<'py, PyAny>;
    type Error = PyErr;

    fn into_pyobject(self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        (*self).into_pyobject(py)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn epoch_from_civil() {
        let d = |year, month, day| Date { year, month, day };
        assert_eq!(days_from_civil(d(1970, 1, 1)), 0);
        assert_eq!(days_from_civil(d(2000, 3, 1)), 11_017);
        assert_eq!(days_from_civil(d(1, 1, 1)), -719_162);
        assert_eq!(days_from_civil(d(9999, 12, 31)), 2_932_896);
        let odt = OffsetDateTime {
            local: PlainDateTime {
                date: d(2024, 2, 29),
                time: Time {
                    hour: 3,
                    minute: 4,
                    second: 5,
                    nanosecond: 7,
                },
            },
            offset_second: -3_723,
        };
        assert_eq!(
            odt.instant(),
            Instant {
                epoch_second: 1_709_175_845 + 3_723,
                nanosecond: 7
            }
        );
    }
}
