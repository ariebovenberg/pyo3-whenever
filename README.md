# pyo3-whenever

PyO3 conversions between the [whenever](https://whenever.readthedocs.io)
datetime library and [jiff](https://docs.rs/jiff), through whenever's C API.

Wrap a jiff type in `Whenever` to take or return a whenever object. The
nanoseconds and the time zone ID survive the conversion. PyO3's own jiff
conversions go through `datetime` objects and lose both.

```rust
use jiff::{Timestamp, Zoned, tz::TimeZone};
use pyo3::prelude::*;
use pyo3_whenever::{Whenever, WheneverOrDatetime};

#[pyfunction]
fn in_utc(instant: Whenever<Timestamp>) -> Whenever<Zoned> {
    Whenever(instant.0.to_zoned(TimeZone::UTC))
}

// Takes a `whenever.Instant` or a `datetime.datetime` with a tzinfo.
#[pyfunction]
fn epoch_second(instant: WheneverOrDatetime<Timestamp>) -> i64 {
    instant.0.as_second()
}
```

## How the types map

`Whenever<T>` converts between a whenever class and the jiff type `T`.

| whenever            | jiff                                | Notes   |
|---------------------|-------------------------------------|---------|
| `Date`              | `civil::Date`                       | 1       |
| `Time`              | `civil::Time`                       |         |
| `PlainDateTime`     | `civil::DateTime`                   | 1       |
| `Instant`           | `Timestamp`                         | 2       |
| `TimeDelta`         | `SignedDuration`                    | 3       |
| `ZonedDateTime`     | `Zoned` in an IANA time zone        | 2, 4, 5 |
| `OffsetDateTime`    | `Zoned` in a fixed-offset time zone | 2, 6, 7 |
| `ItemizedDelta`     | `Span`                              | 8, 9    |
| `ItemizedDateDelta` | `Span`                              | 8       |

A value that does not convert raises `ValueError`.

1. jiff's civil types reach year -9999, and whenever starts at year 1. A
   jiff value before year 1 does not convert.
2. jiff's `Timestamp` ends at `9999-12-30T22:00:00.999999999Z`, about 26
   hours before whenever's `Instant.MAX`. whenever's `Instant` starts at
   year 1, and jiff's `Timestamp` starts at year -9999. An instant outside
   the range of the other library does not convert.
3. A `SignedDuration` beyond `TimeDelta.MIN` or `TimeDelta.MAX` does not
   convert.
4. The instant and the time zone ID convert. Each library computes the
   local time from its own time zone data, so the local times differ when
   the two databases disagree. A time zone ID that the other library does
   not know does not convert.
5. A `ZonedDateTime` without a time zone ID does not convert. Neither does
   a `Zoned` in a time zone that has no IANA name and no fixed offset,
   such as a POSIX time zone.
6. whenever's offsets stop at ±23:59:59, and jiff's reach ±25:59:59. A
   `Zoned` with a larger offset does not convert.
7. jiff has one value for UTC and for a fixed offset of zero, and that
   value has the IANA name `UTC`. An `OffsetDateTime` with offset `+00:00`
   therefore comes back as a `ZonedDateTime` in `UTC`. Every other offset
   round-trips as an `OffsetDateTime`.
8. A `Span` has no absent components, only zero ones. An absent component
   becomes zero, and only the nonzero components come back, so a zero
   `Span` becomes an empty `ItemizedDelta`. A `Span` always comes back as
   an `ItemizedDelta`.
9. jiff's milliseconds and microseconds become seconds and nanoseconds.
   jiff's limits per unit are about twice whenever's, so a large `Span`
   does not convert.

`Whenever` accepts exactly the whenever class, and it is the only wrapper
that returns a value.

## Accept `datetime` objects too

`WheneverOrDatetime<T>` takes a whenever object or its counterpart in the
`datetime` module.

| jiff              | `datetime` class                |
|-------------------|---------------------------------|
| `civil::Date`     | `datetime.date`                 |
| `civil::Time`     | `datetime.time`                 |
| `civil::DateTime` | `datetime.datetime`, no tzinfo  |
| `Timestamp`       | `datetime.datetime` with tzinfo |
| `SignedDuration`  | `datetime.timedelta`            |
| `Zoned`           | `datetime.datetime` with tzinfo |
| `Span`            | `datetime.timedelta`            |

`WheneverOrDatetime` is for arguments. It converts a `datetime` object as
PyO3's `jiff-02` feature does, subclasses included: a `datetime.datetime`
is a `datetime.date`, so it extracts as a `civil::Date` and loses its time.

## Records

Each of whenever's classes in the table also has a plain Rust **record** in
`pyo3_whenever::records` that implements `FromPyObject` and `IntoPyObject`:
`Date`, `Time`, `PlainDateTime`, `Instant`, `OffsetDateTime`,
`ZonedDateTime`, `TimeDelta`, `ItemizedDelta`, and `ItemizedDateDelta`. Use the records to convert without jiff,
to accept one class only, or to choose how a `ZonedDateTime` resolves
against the time zone data.

```rust
use pyo3::prelude::*;
use pyo3_whenever::records::ZonedDateTime;

#[pyfunction]
fn tz_id(zoned: ZonedDateTime) -> Option<String> {
    zoned.tz_id
}
```

A `ZonedDateTime` record holds what whenever stores: the local fields, the
offset, and the time zone ID. Building an object from one checks the
offset against whenever's time zone data. `to_pyobject_with()` takes the
`offset_mismatch` and `disambiguation` policies, and `Instant::to_tz()`
places an instant in a zone.
`to_zoned_checked()` converts to a jiff `Zoned` and fails when jiff's time
zone data disagrees with the stored offset.

An itemized delta record has an `Option` per component, so it keeps the
difference between an absent component and a zero one.

## Features

No feature is on by default. Without one, the crate has the records only.

- `jiff-02`: `Whenever`, `WheneverOrDatetime`, and the conversions between
  the records and the types of jiff 0.2. It enables PyO3's `jiff-02`
  feature, which enables jiff's default features. Those include jiff's
  time zone database features.

```toml
[dependencies]
pyo3-whenever = { version = "0.1.0-alpha.1", features = ["jiff-02"] }
```

jiff 1.0 is not supported yet. `WheneverOrDatetime` depends on PyO3's
conversions from `datetime` objects, and PyO3 has those for jiff 0.2 only.

## How it works

When whenever's Rust extension is loaded, conversions go through the
`whenever.C_API_v1` capsule: a table of C functions that read and build
whenever objects with no Python call per value. On the pure-Python backend
(PyPy, or a source install without the extension) they fall back to
`to_bytes()` and `from_bytes()`, one Python call per value, with the same
results. `uses_c_api()` tells which path is active.

`ItemizedDelta` and `ItemizedDateDelta` are Python classes on both
backends. They convert as pickle does: through `__reduce__()` and
whenever's unpickle functions.

Requires whenever 1.0 or later, and PyO3 0.29.

## Testing

The tests embed the interpreter that built the crate and import whenever
from a checkout next to this one, with its extension built in place:

```
PYO3_PYTHON=../whenever/.venv/bin/python make test
```

`make test` runs the suite against both backends.
