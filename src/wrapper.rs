//! [`Whenever`] and [`WheneverOrDatetime`]: jiff types as the arguments and
//! return values of Python-facing functions.

use crate::{
    Error,
    records::{self, wrong_type},
};
use core::fmt::Display;
use jiff::{SignedDuration, Span, Timestamp, Zoned, civil};
use pyo3::{
    PyTypeCheck,
    exceptions::PyValueError,
    prelude::*,
    types::{PyDate, PyDateTime, PyDelta, PyTime},
};

/// A jiff value that crosses into Python as a whenever object.
///
/// | `T`                | whenever class                         |
/// |--------------------|----------------------------------------|
/// | [`civil::Date`]    | `Date`                                 |
/// | [`civil::Time`]    | `Time`                                 |
/// | [`civil::DateTime`]| `PlainDateTime`                        |
/// | [`Timestamp`]      | `Instant`                              |
/// | [`SignedDuration`] | `TimeDelta`                            |
/// | [`Zoned`]          | `ZonedDateTime` or `OffsetDateTime`    |
/// | [`Span`]           | `ItemizedDelta` or `ItemizedDateDelta` |
///
/// Extraction accepts exactly the class in the table, not a subclass and
/// not a `datetime` object. To accept `datetime` objects too, use
/// [`WheneverOrDatetime`].
///
/// ```no_run
/// use jiff::{Timestamp, Zoned, tz::TimeZone};
/// use pyo3::prelude::*;
/// use pyo3_whenever::Whenever;
///
/// #[pyfunction]
/// fn in_utc(instant: Whenever<Timestamp>) -> Whenever<Zoned> {
///     Whenever(instant.0.to_zoned(TimeZone::UTC))
/// }
/// ```
///
/// # Range
///
/// jiff and whenever do not cover the same range. A value that the other
/// library cannot hold raises `ValueError`:
///
/// - jiff's `Timestamp` ends about 26 hours before whenever's
///   `Instant.MAX`.
/// - whenever starts at year 1; jiff's civil types reach year -9999.
/// - whenever's offsets stop at ±23:59:59; jiff's reach ±25:59:59.
///
/// # `Zoned`
///
/// A `ZonedDateTime` becomes a `Zoned` in the same IANA time zone. An
/// `OffsetDateTime` becomes a `Zoned` in a fixed-offset time zone. Both
/// keep the instant: each library computes the local time from its own
/// time zone data, so the local times differ when the two databases
/// disagree.
///
/// On the way back, the time zone of the `Zoned` decides the class:
///
/// - A time zone with an IANA name gives a `ZonedDateTime`.
/// - A fixed offset gives an `OffsetDateTime`.
/// - Any other time zone, such as a POSIX one, raises `ValueError`. So does
///   a `ZonedDateTime` without a time zone ID on the way in.
///
/// jiff has one value for UTC and for a fixed offset of zero, and that
/// value has the IANA name `UTC`. An `OffsetDateTime` with offset `+00:00`
/// therefore comes back as a `ZonedDateTime` in `UTC`. Every other offset
/// round-trips as an `OffsetDateTime`.
///
/// # `Span`
///
/// A `Span` comes back as an `ItemizedDelta`. It has no absent components,
/// only zero ones. An absent component
/// becomes zero, and only the nonzero components come back, so a zero
/// `Span` becomes an empty `ItemizedDelta`. Milliseconds and microseconds
/// become seconds and nanoseconds. A `Span` beyond whenever's limits per
/// unit raises `ValueError`.
#[repr(transparent)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Whenever<T>(pub T);

/// A jiff value extracted from a whenever object or from its counterpart
/// in the `datetime` module.
///
/// | `T`                | `datetime` class           |
/// |--------------------|----------------------------|
/// | [`civil::Date`]    | `date`                     |
/// | [`civil::Time`]    | `time`                     |
/// | [`civil::DateTime`]| `datetime` without tzinfo  |
/// | [`Timestamp`]      | `datetime` with tzinfo     |
/// | [`SignedDuration`] | `timedelta`                |
/// | [`Zoned`]          | `datetime` with tzinfo     |
/// | [`Span`]           | `timedelta`                |
///
/// A whenever object converts as [`Whenever`] converts it. A `datetime`
/// object converts as PyO3's `jiff-02` feature converts it, subclasses
/// included: a `datetime.datetime` is a `datetime.date`, so it extracts as
/// a [`civil::Date`] and loses its time.
///
/// This type is for arguments only. To return a value, choose the Python
/// type: [`Whenever`] for a whenever object, the bare jiff type for a
/// `datetime` object.
///
/// ```no_run
/// use jiff::Timestamp;
/// use pyo3::prelude::*;
/// use pyo3_whenever::WheneverOrDatetime;
///
/// #[pyfunction]
/// fn epoch_second(instant: WheneverOrDatetime<Timestamp>) -> i64 {
///     instant.0.as_second()
/// }
/// ```
#[repr(transparent)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct WheneverOrDatetime<T>(pub T);

mod sealed {
    pub trait Sealed {}
}

/// The jiff types that [`Whenever`] and [`WheneverOrDatetime`] convert.
/// Sealed: the list in the documentation of [`Whenever`] is complete.
pub trait Convert: sealed::Sealed + Sized {
    /// The whenever classes that extract to this type.
    #[doc(hidden)]
    const WHENEVER: &'static [&'static str];

    /// The `datetime` class that extracts to this type.
    #[doc(hidden)]
    const DATETIME: &'static str;

    /// `None` when the object is of none of the whenever classes.
    #[doc(hidden)]
    fn from_whenever(ob: &Bound<'_, PyAny>) -> PyResult<Option<Self>>;

    /// `None` when the object is not an instance of the `datetime` class.
    #[doc(hidden)]
    fn from_datetime(ob: &Bound<'_, PyAny>) -> PyResult<Option<Self>>;

    #[doc(hidden)]
    fn to_whenever<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>>;
}

impl<'py, T: Convert> FromPyObject<'_, 'py> for Whenever<T> {
    type Error = PyErr;

    fn extract(ob: Borrowed<'_, 'py, PyAny>) -> PyResult<Self> {
        T::from_whenever(&ob)?
            .map(Self)
            .ok_or_else(|| wrong_type(&ob, &one_of(T::WHENEVER)))
    }
}

impl<'py, T: Convert> IntoPyObject<'py> for &Whenever<T> {
    type Target = PyAny;
    type Output = Bound<'py, PyAny>;
    type Error = PyErr;

    fn into_pyobject(self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        self.0.to_whenever(py)
    }
}

impl<'py, T: Convert> IntoPyObject<'py> for Whenever<T> {
    type Target = PyAny;
    type Output = Bound<'py, PyAny>;
    type Error = PyErr;

    fn into_pyobject(self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        self.0.to_whenever(py)
    }
}

impl<'py, T: Convert> FromPyObject<'_, 'py> for WheneverOrDatetime<T> {
    type Error = PyErr;

    fn extract(ob: Borrowed<'_, 'py, PyAny>) -> PyResult<Self> {
        match T::from_whenever(&ob)? {
            Some(v) => Ok(Self(v)),
            None => T::from_datetime(&ob)?.map(Self).ok_or_else(|| {
                let accepted: Vec<&str> =
                    T::WHENEVER.iter().copied().chain([T::DATETIME]).collect();
                wrong_type(&ob, &one_of(&accepted))
            }),
        }
    }
}

/// "a", "a or b", "a, b, or c".
fn one_of(names: &[&str]) -> String {
    match names {
        [] => String::new(),
        [a] => (*a).to_owned(),
        [a, b] => format!("{a} or {b}"),
        [head @ .., last] => format!("{}, or {last}", head.join(", ")),
    }
}

/// A whenever object of the right class whose value jiff cannot hold.
fn cannot_convert_from(class: &str, e: impl Display) -> PyErr {
    PyValueError::new_err(format!("cannot convert this {class}: {e}"))
}

/// A jiff value that the whenever class cannot hold.
fn cannot_convert_to(class: &str, e: impl Display) -> PyErr {
    PyValueError::new_err(format!("cannot convert to a {class}: {e}"))
}

/// PyO3's conversion from the `datetime` class `P`, or `None` when the
/// object is not an instance of `P`.
fn from_datetime<'py, T, P>(ob: &Bound<'py, PyAny>) -> PyResult<Option<T>>
where
    T: for<'a> FromPyObject<'a, 'py, Error = PyErr>,
    P: PyTypeCheck,
{
    ob.is_instance_of::<P>().then(|| ob.extract()).transpose()
}

macro_rules! via_record {
    ($jiff:ty, $record:ident, $whenever:literal, $pytype:ty, $datetime:literal) => {
        impl sealed::Sealed for $jiff {}

        impl Convert for $jiff {
            const WHENEVER: &'static [&'static str] = &[$whenever];
            const DATETIME: &'static str = $datetime;

            fn from_whenever(ob: &Bound<'_, PyAny>) -> PyResult<Option<Self>> {
                records::$record::try_extract(ob)?
                    .map(|r| Self::try_from(r).map_err(|e| cannot_convert_from($whenever, e)))
                    .transpose()
            }

            fn from_datetime(ob: &Bound<'_, PyAny>) -> PyResult<Option<Self>> {
                from_datetime::<Self, $pytype>(ob)
            }

            fn to_whenever<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
                records::$record::try_from(*self)
                    .map_err(Error::from)
                    .map_err(|e| cannot_convert_to($whenever, e))?
                    .into_pyobject(py)
            }
        }
    };
}

via_record!(civil::Date, Date, "whenever.Date", PyDate, "datetime.date");
via_record!(civil::Time, Time, "whenever.Time", PyTime, "datetime.time");
via_record!(
    civil::DateTime,
    PlainDateTime,
    "whenever.PlainDateTime",
    PyDateTime,
    "datetime.datetime"
);
via_record!(
    Timestamp,
    Instant,
    "whenever.Instant",
    PyDateTime,
    "datetime.datetime"
);
via_record!(
    SignedDuration,
    TimeDelta,
    "whenever.TimeDelta",
    PyDelta,
    "datetime.timedelta"
);

const ZONED: &str = "whenever.ZonedDateTime";
const OFFSET: &str = "whenever.OffsetDateTime";

impl sealed::Sealed for Zoned {}

impl Convert for Zoned {
    const WHENEVER: &'static [&'static str] = &[ZONED, OFFSET];
    const DATETIME: &'static str = "datetime.datetime";

    fn from_whenever(ob: &Bound<'_, PyAny>) -> PyResult<Option<Self>> {
        match records::ZonedDateTime::try_extract_with(ob, crate::jiff::zoned)? {
            Some(z) => z.map(Some).map_err(|e| cannot_convert_from(ZONED, e)),
            None => records::OffsetDateTime::try_extract(ob)?
                .map(|o| Self::try_from(o).map_err(|e| cannot_convert_from(OFFSET, e)))
                .transpose(),
        }
    }

    fn from_datetime(ob: &Bound<'_, PyAny>) -> PyResult<Option<Self>> {
        from_datetime::<Self, PyDateTime>(ob)
    }

    fn to_whenever<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let tz = self.time_zone();
        // The name comes first: jiff has one value for UTC and for a fixed
        // offset of zero, and that value has both a name and an offset.
        match (tz.iana_name(), tz.to_fixed_offset()) {
            (Some(tz_id), _) => {
                let instant = records::Instant::try_from(self.timestamp())
                    .map_err(|e| cannot_convert_to(ZONED, e))?;
                instant.to_tz(py, tz_id)
            }
            (None, Ok(_)) => records::OffsetDateTime::try_from(self)
                .map_err(|e| cannot_convert_to(OFFSET, e))?
                .into_pyobject(py),
            (None, Err(_)) => Err(PyValueError::new_err(format!(
                "cannot convert to a {ZONED} or {OFFSET}: \
                 the time zone has no IANA name and no fixed offset"
            ))),
        }
    }
}

const IDELTA: &str = "whenever.ItemizedDelta";
const IDATE_DELTA: &str = "whenever.ItemizedDateDelta";

impl sealed::Sealed for Span {}

impl Convert for Span {
    const WHENEVER: &'static [&'static str] = &[IDELTA, IDATE_DELTA];
    const DATETIME: &'static str = "datetime.timedelta";

    fn from_whenever(ob: &Bound<'_, PyAny>) -> PyResult<Option<Self>> {
        match records::ItemizedDelta::try_extract(ob)? {
            Some(d) => Self::try_from(d)
                .map(Some)
                .map_err(|e| cannot_convert_from(IDELTA, e)),
            None => records::ItemizedDateDelta::try_extract(ob)?
                .map(|d| Self::try_from(d).map_err(|e| cannot_convert_from(IDATE_DELTA, e)))
                .transpose(),
        }
    }

    fn from_datetime(ob: &Bound<'_, PyAny>) -> PyResult<Option<Self>> {
        from_datetime::<Self, PyDelta>(ob)
    }

    fn to_whenever<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        records::ItemizedDelta::from(*self).into_pyobject(py)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names() {
        assert_eq!(one_of(&["a"]), "a");
        assert_eq!(one_of(&["a", "b"]), "a or b");
        assert_eq!(one_of(&["a", "b", "c"]), "a, b, or c");
    }
}
