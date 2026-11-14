//! The records: whenever's seven C API types as plain Rust values, with
//! their conversions to and from Python objects.

use crate::{Backend, backend, bytes, capi};
use core::{ffi::c_int, mem::MaybeUninit};
use pyo3::{
    exceptions::{PyTypeError, PyValueError},
    prelude::*,
    types::{PyBytes, PyDict},
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

    /// The instant in the named time zone, with the local fields computed
    /// by whenever's time zone data. Raises `TimeZoneNotFoundError` for an
    /// unknown ID.
    pub fn from_instant<'py>(
        py: Python<'py>,
        instant: Instant,
        tz_id: &str,
    ) -> PyResult<Bound<'py, PyAny>> {
        match backend(py)? {
            Backend::Capsule { table: t, .. } => {
                let tz_id_len = u8::try_from(tz_id.len())
                    .map_err(|_| PyValueError::new_err("time zone ID too long"))?;
                let rec = capi::Instant::from(instant);
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
                    (PyBytes::new(py, &bytes::encode_instant(instant)),),
                )?
                .call_method1("to_tz", (tz_id,)),
        }
    }

    /// The record as written, resolved against whenever's time zone data
    /// under the given policies. [`IntoPyObject`] uses
    /// [`OffsetMismatch::Raise`], which never consults the disambiguation.
    pub fn into_pyobject_with<'py>(
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

impl From<capi::ZonedDateTime> for ZonedDateTime {
    /// The ID is copied out of the object, which the extractor's caller
    /// keeps alive for the call.
    fn from(r: capi::ZonedDateTime) -> Self {
        let tz_id = (!r.tz_id.is_null()).then(|| {
            // SAFETY: a non-null ID is `tz_id_len` ASCII bytes borrowed from a live object.
            let bytes =
                unsafe { core::slice::from_raw_parts(r.tz_id.cast::<u8>(), r.tz_id_len as usize) };
            String::from_utf8_lossy(bytes).into_owned()
        });
        Self {
            odt: r.odt.into(),
            tz_id,
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

fn wrong_type(ob: &Bound<'_, PyAny>, expected: &str) -> PyErr {
    PyTypeError::new_err(format!(
        "expected a whenever.{expected}, got {}",
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
/// without one.
fn extract<'py, T, R: Copy>(
    ob: &Bound<'py, PyAny>,
    name: &str,
    slot: impl Fn(&capi::Table) -> capi::Extract<R>,
    py_type: impl Fn(&crate::PyTypes) -> &Py<pyo3::types::PyType>,
    fallback: impl Fn(&Bound<'py, PyAny>) -> PyResult<T>,
) -> PyResult<T>
where
    T: From<R>,
{
    match backend(ob.py())? {
        Backend::Capsule { table: t, .. } => {
            let mut rec = MaybeUninit::<R>::uninit();
            // SAFETY: the table is the capsule's; `ob` is a live object; `rec` is ours to fill.
            match unsafe { slot(t)(*t, ob.as_ptr(), rec.as_mut_ptr()) } {
                // SAFETY: 0 means the extractor filled the record.
                0 => Ok(unsafe { rec.assume_init() }.into()),
                _ => Err(wrong_type(ob, name)),
            }
        }
        Backend::Python(types) => {
            if !ob.get_type().is(py_type(types)) {
                return Err(wrong_type(ob, name));
            }
            fallback(ob)
        }
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
        impl<'py> FromPyObject<'_, 'py> for $typ {
            type Error = PyErr;

            fn extract(ob: Borrowed<'_, 'py, PyAny>) -> PyResult<Self> {
                extract(
                    &ob,
                    $name,
                    |t| t.$extract,
                    |types| &types.$slot,
                    |ob| from_to_bytes(ob, $decode),
                )
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
    "Date",
    date_extract,
    date_new,
    date,
    bytes::decode_date,
    bytes::encode_date
);
conversions!(
    Time,
    "Time",
    time_extract,
    time_new,
    time,
    bytes::decode_time,
    bytes::encode_time
);
conversions!(
    PlainDateTime,
    "PlainDateTime",
    plain_datetime_extract,
    plain_datetime_new,
    plain_datetime,
    bytes::decode_plain,
    bytes::encode_plain
);
conversions!(
    Instant,
    "Instant",
    instant_extract,
    instant_new,
    instant,
    bytes::decode_instant,
    bytes::encode_instant
);
conversions!(
    OffsetDateTime,
    "OffsetDateTime",
    offset_datetime_extract,
    offset_datetime_new,
    offset_datetime,
    bytes::decode_offset,
    bytes::encode_offset
);
conversions!(
    TimeDelta,
    "TimeDelta",
    time_delta_extract,
    time_delta_new,
    time_delta,
    bytes::decode_time_delta,
    bytes::encode_time_delta
);

impl<'py> FromPyObject<'_, 'py> for ZonedDateTime {
    type Error = PyErr;

    fn extract(ob: Borrowed<'_, 'py, PyAny>) -> PyResult<Self> {
        extract(
            &ob,
            "ZonedDateTime",
            |t| t.zoned_datetime_extract,
            |types| &types.zoned_datetime,
            // `to_bytes()` refuses a system time zone without an ID, which
            // the capsule extracts with a null ID.
            |ob| {
                if ob.getattr("tz_id")?.is_none() {
                    let odt = ob.call_method0("to_fixed_offset")?;
                    Ok(ZonedDateTime {
                        odt: from_to_bytes(&odt, bytes::decode_offset)?,
                        tz_id: None,
                    })
                } else {
                    from_to_bytes(ob, bytes::decode_zoned)
                }
            },
        )
    }
}

impl<'py> IntoPyObject<'py> for &ZonedDateTime {
    type Target = PyAny;
    type Output = Bound<'py, PyAny>;
    type Error = PyErr;

    fn into_pyobject(self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        self.into_pyobject_with(py, OffsetMismatch::Raise, Disambiguation::Compatible)
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
