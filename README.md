# pyo3-whenever

PyO3 conversions for the [whenever](https://whenever.readthedocs.io) datetime
library, through its C API.

Each of whenever's seven core types has a plain Rust **record** that
implements `FromPyObject` and `IntoPyObject`: `Date`, `Time`,
`PlainDateTime`, `Instant`, `OffsetDateTime`, `ZonedDateTime`, and
`TimeDelta`. With the `jiff-02` feature, the records convert to and from
[jiff](https://docs.rs/jiff)'s types, with the nanoseconds and time zone
IDs that PyO3's own jiff conversions lose by going through `datetime`.

```rust
use pyo3::prelude::*;
use pyo3_whenever::{Instant, ZonedDateTime};

#[pyfunction]
fn epoch_second(instant: Instant) -> i64 {
    instant.epoch_second
}

#[pyfunction]
fn tz_id(zoned: ZonedDateTime) -> Option<String> {
    zoned.tz_id
}
```

## How it works

When whenever's Rust extension is loaded, conversions go through the
`whenever.C_API_v1` capsule: a table of C functions that read and build
whenever objects with no Python call per value. On the pure-Python backend
(PyPy, or a source install without the extension) they fall back to
`to_bytes()` and `from_bytes()`, one Python call per value, with the same
results. `uses_c_api()` tells which path is active.

A `ZonedDateTime` converts as whenever stores it: local fields, offset,
and time zone ID. Building one checks the offset against whenever's time
zone data. `into_pyobject_with()` takes the `offset_mismatch` and
`disambiguation` policies, and `ZonedDateTime::from_instant()` places an
instant in a zone. Converting to a jiff `Zoned` keeps the instant.
`to_zoned_checked()` fails instead when jiff's time zone data disagrees
with the stored offset. Both read the time zone through jiff's own
database, so the consumer's `jiff` dependency decides which `tzdb-*`
features are on; this crate enables none.

Requires whenever 1.0 or later, and PyO3 0.29.

## Testing

The tests embed the interpreter that built the crate and import whenever
from a checkout next to this one, with its extension built in place:

```
PYO3_PYTHON=../whenever/.venv/bin/python make test
```

`make test` runs the suite against both backends.
