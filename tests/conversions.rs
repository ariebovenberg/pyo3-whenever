//! Round trips against the whenever installed next to this crate, on both
//! backends: `PYO3_WHENEVER_TEST_PURE=1` selects the pure-Python one.

use pyo3::{prelude::*, types::PyDict};
use pyo3_whenever::*;
use std::sync::Once;

static SETUP: Once = Once::new();

fn setup() {
    SETUP.call_once(|| {
        Python::initialize();
        Python::attach(|py| {
            let sys = py.import("sys").unwrap();
            // A whenever checkout next to this one wins over an installed one.
            let pysrc = std::env::var("WHENEVER_PYSRC").unwrap_or_else(|_| {
                concat!(env!("CARGO_MANIFEST_DIR"), "/../whenever/pysrc").into()
            });
            if std::path::Path::new(&pysrc).is_dir() {
                sys.getattr("path")
                    .unwrap()
                    .call_method1("insert", (0, pysrc))
                    .unwrap();
            }
            if std::env::var_os("PYO3_WHENEVER_TEST_PURE").is_some() {
                sys.getattr("modules")
                    .unwrap()
                    .set_item("whenever._whenever", py.None())
                    .unwrap();
            }
        });
    });
}

fn whenever<'py>(py: Python<'py>) -> Bound<'py, PyModule> {
    py.import("whenever").unwrap()
}

/// `Python::attach`, after the one-time setup.
fn attach<R>(f: impl for<'py> FnOnce(Python<'py>) -> R) -> R {
    setup();
    Python::attach(f)
}

fn eval<'py>(py: Python<'py>, code: &str) -> Bound<'py, PyAny> {
    let globals = PyDict::new(py);
    globals.set_item("w", whenever(py)).unwrap();
    py.eval(&std::ffi::CString::new(code).unwrap(), Some(&globals), None)
        .unwrap()
}

fn round_trip<T>(code: &str) -> T
where
    T: for<'a, 'py> FromPyObject<'a, 'py, Error = PyErr>
        + for<'py> IntoPyObject<'py, Target = PyAny, Output = Bound<'py, PyAny>, Error = PyErr>
        + Clone,
{
    attach(|py| {
        let obj = eval(py, code);
        let value: T = obj.extract().unwrap();
        let back = value.clone().into_pyobject(py).unwrap();
        assert!(obj.eq(&back).unwrap(), "{code}: {back}");
        if obj.hasattr("strict_eq").unwrap() {
            assert!(
                obj.call_method1("strict_eq", (&back,))
                    .unwrap()
                    .is_truthy()
                    .unwrap()
            );
        }
        value
    })
}

#[test]
fn backend_matches_environment() {
    attach(|py| {
        let loaded = whenever(py)
            .getattr("_EXTENSION_LOADED")
            .unwrap()
            .is_truthy()
            .unwrap();
        assert_eq!(
            loaded,
            std::env::var_os("PYO3_WHENEVER_TEST_PURE").is_none()
        );
        assert_eq!(uses_c_api(py).unwrap(), loaded);
    });
}

#[test]
fn date() {
    let d: Date = round_trip("w.Date(2024, 2, 29)");
    assert_eq!(
        d,
        Date {
            year: 2024,
            month: 2,
            day: 29
        }
    );
    round_trip::<Date>("w.Date.MIN");
    round_trip::<Date>("w.Date.MAX");
}

#[test]
fn time() {
    let t: Time = round_trip("w.Time(3, 4, 5, nanosecond=600_700_800)");
    assert_eq!(
        t,
        Time {
            hour: 3,
            minute: 4,
            second: 5,
            nanosecond: 600_700_800
        }
    );
    round_trip::<Time>("w.Time.MAX");
}

#[test]
fn plain_datetime() {
    let p: PlainDateTime = round_trip("w.PlainDateTime(2024, 2, 29, 3, 4, 5, nanosecond=7)");
    assert_eq!(p.date.day, 29);
    assert_eq!(p.time.nanosecond, 7);
    round_trip::<PlainDateTime>("w.PlainDateTime.MIN");
    round_trip::<PlainDateTime>("w.PlainDateTime.MAX");
}

#[test]
fn instant() {
    let i: Instant = round_trip("w.Instant.from_utc(2024, 2, 29, 3, 4, 5, nanosecond=7)");
    assert_eq!(
        i,
        Instant {
            epoch_second: 1_709_175_845,
            nanosecond: 7
        }
    );
    assert_eq!(
        round_trip::<Instant>("w.Instant.MIN").epoch_second,
        -62_135_596_800
    );
    assert_eq!(
        round_trip::<Instant>("w.Instant.MAX").nanosecond,
        999_999_999
    );
}

#[test]
fn offset_datetime() {
    let o: OffsetDateTime =
        round_trip("w.OffsetDateTime(2024, 2, 29, 3, 4, 5, offset=w.TimeDelta(seconds=-3723))");
    assert_eq!(o.offset_second, -3723);
    assert_eq!(
        o.instant(),
        Instant {
            epoch_second: 1_709_175_845 + 3723,
            nanosecond: 0
        }
    );
}

#[test]
fn time_delta() {
    let d: TimeDelta = round_trip("w.TimeDelta(seconds=-12_345, nanoseconds=678_901_234)");
    assert_eq!(
        d,
        TimeDelta {
            second: -12_345,
            nanosecond: 678_901_234
        }
    );
    assert_eq!(
        round_trip::<TimeDelta>("w.TimeDelta(nanoseconds=-1)"),
        TimeDelta {
            second: -1,
            nanosecond: 999_999_999
        }
    );
    round_trip::<TimeDelta>("w.TimeDelta.MIN");
    round_trip::<TimeDelta>("w.TimeDelta.MAX");
}

#[test]
fn zoned_datetime() {
    let z: ZonedDateTime =
        round_trip("w.ZonedDateTime(2024, 2, 29, 3, 4, 5, tz='europe/amsterdam')");
    assert_eq!(z.tz_id.as_deref(), Some("Europe/Amsterdam"));
    assert_eq!(z.odt.offset_second, 3600);
    assert_eq!(z.instant().epoch_second, 1_709_175_845 - 3600);
    // a repeated local time keeps its occurrence through the offset
    round_trip::<ZonedDateTime>(
        "w.ZonedDateTime(2023, 10, 29, 2, 30, tz='Europe/Amsterdam', disambiguation='later')",
    );
}

#[test]
fn zoned_datetime_without_tz_id() {
    attach(|py| {
        let environ = py.import("os").unwrap().getattr("environ").unwrap();
        let before = environ.call_method1("get", ("TZ",)).unwrap();
        environ
            .set_item("TZ", "CET-1CEST,M3.5.0,M10.5.0/3")
            .unwrap();
        whenever(py).call_method0("reset_system_tz").unwrap();
        let obj = eval(py, "w.ZonedDateTime(2024, 2, 29, 3, 4, 5, tz=w.SYSTEM_TZ)");
        if before.is_none() {
            environ.del_item("TZ").unwrap();
        } else {
            environ.set_item("TZ", before).unwrap();
        }
        whenever(py).call_method0("reset_system_tz").unwrap();

        let z: ZonedDateTime = obj.extract().unwrap();
        assert_eq!(z.tz_id, None);
        assert_eq!(z.odt.offset_second, 3600);
        assert_eq!(z.odt.local.time.hour, 3);
        let err = z.into_pyobject(py).unwrap_err();
        assert!(err.to_string().contains("without a time zone ID"), "{err}");
    });
}

#[test]
fn zoned_datetime_policies() {
    attach(|py| {
        let z: ZonedDateTime = eval(py, "w.ZonedDateTime(2024, 7, 1, 12, tz='Europe/Amsterdam')")
            .extract()
            .unwrap();
        let stale = ZonedDateTime {
            odt: OffsetDateTime {
                offset_second: 3600,
                ..z.odt
            },
            ..z
        };
        let err = (&stale).into_pyobject(py).unwrap_err();
        assert!(err.is_instance(py, &whenever(py).getattr("InvalidOffsetError").unwrap()));
        let kept = stale
            .into_pyobject_with(py, OffsetMismatch::KeepInstant, Disambiguation::Compatible)
            .unwrap();
        assert_eq!(kept.getattr("hour").unwrap().extract::<u8>().unwrap(), 13);
        let kept = stale
            .into_pyobject_with(py, OffsetMismatch::KeepLocal, Disambiguation::Raise)
            .unwrap();
        assert_eq!(kept.getattr("hour").unwrap().extract::<u8>().unwrap(), 12);
    });
}

#[test]
fn zoned_datetime_from_instant() {
    attach(|py| {
        let instant = Instant {
            epoch_second: 1_709_175_845,
            nanosecond: 7,
        };
        let z = ZonedDateTime::from_instant(py, instant, "Europe/Amsterdam").unwrap();
        let expected = eval(
            py,
            "w.ZonedDateTime(2024, 2, 29, 4, 4, 5, nanosecond=7, tz='Europe/Amsterdam')",
        );
        assert!(
            z.call_method1("strict_eq", (expected,))
                .unwrap()
                .is_truthy()
                .unwrap()
        );
        let err = ZonedDateTime::from_instant(py, instant, "Nope/Zone").unwrap_err();
        assert!(err.is_instance(py, &whenever(py).getattr("TimeZoneNotFoundError").unwrap()));
    });
}

#[test]
fn wrong_type() {
    attach(|py| {
        let obj = eval(py, "w.Time(1)");
        let err = obj.extract::<Date>().unwrap_err();
        assert!(err.is_instance_of::<pyo3::exceptions::PyTypeError>(py));
        assert!(err.to_string().contains("whenever.Date"), "{err}");
        assert!(eval(py, "3").extract::<Instant>().is_err());
        assert!(eval(py, "None").extract::<ZonedDateTime>().is_err());
    });
}

#[test]
fn invalid_record() {
    attach(|py| {
        let err = Date {
            year: 2023,
            month: 2,
            day: 29,
        }
        .into_pyobject(py)
        .unwrap_err();
        assert!(
            err.is_instance_of::<pyo3::exceptions::PyValueError>(py),
            "{err:?}"
        );
        let err = Instant {
            epoch_second: 0,
            nanosecond: 1_000_000_000,
        }
        .into_pyobject(py)
        .unwrap_err();
        assert!(
            err.is_instance_of::<pyo3::exceptions::PyValueError>(py),
            "{err:?}"
        );
        let err = ZonedDateTime {
            odt: OffsetDateTime {
                local: PlainDateTime {
                    date: Date {
                        year: 2024,
                        month: 1,
                        day: 1,
                    },
                    time: Time {
                        hour: 0,
                        minute: 0,
                        second: 0,
                        nanosecond: 0,
                    },
                },
                offset_second: 0,
            },
            tz_id: None,
        }
        .into_pyobject(py)
        .unwrap_err();
        assert!(err.to_string().contains("time zone ID"), "{err}");
    });
}

#[cfg(feature = "jiff-02")]
mod jiff_conversions {
    use super::*;
    use jiff::{SignedDuration, Timestamp, Zoned, civil, tz::TimeZone};

    #[test]
    fn civil() {
        let p: PlainDateTime = round_trip("w.PlainDateTime(2024, 2, 29, 3, 4, 5, nanosecond=7)");
        let dt = civil::DateTime::try_from(p).unwrap();
        assert_eq!(dt, civil::DateTime::constant(2024, 2, 29, 3, 4, 5, 7));
        assert_eq!(PlainDateTime::try_from(dt).unwrap(), p);
        assert!(PlainDateTime::try_from(civil::DateTime::constant(0, 12, 31, 0, 0, 0, 0)).is_err());
    }

    #[test]
    fn timestamp() {
        let i: Instant = round_trip("w.Instant.from_utc(2024, 2, 29, 3, 4, 5, nanosecond=7)");
        let ts = Timestamp::try_from(i).unwrap();
        assert_eq!(ts, Timestamp::new(1_709_175_845, 7).unwrap());
        assert_eq!(Instant::try_from(ts).unwrap(), i);
        assert!(Timestamp::try_from(round_trip::<Instant>("w.Instant.MAX")).is_err());
    }

    #[test]
    fn duration() {
        let d: TimeDelta = round_trip("w.TimeDelta(seconds=-12_345, nanoseconds=678_901_234)");
        let sd = SignedDuration::try_from(d).unwrap();
        assert_eq!(sd, SignedDuration::new(-12_344, -321_098_766));
        assert_eq!(TimeDelta::try_from(sd).unwrap(), d);
    }

    #[test]
    fn zoned() {
        let z: ZonedDateTime = round_trip("w.ZonedDateTime(2024, 7, 1, 12, tz='Europe/Amsterdam')");
        let jz = Zoned::try_from(&z).unwrap();
        assert_eq!(
            jz.datetime(),
            civil::DateTime::constant(2024, 7, 1, 12, 0, 0, 0)
        );
        assert_eq!(jz.time_zone().iana_name(), Some("Europe/Amsterdam"));
        assert_eq!(ZonedDateTime::try_from(&jz).unwrap(), z);
        assert!(z.to_zoned_checked().is_ok());
        let stale = ZonedDateTime {
            odt: OffsetDateTime {
                offset_second: 3600,
                ..z.odt
            },
            ..z.clone()
        };
        assert!(matches!(
            stale.to_zoned_checked(),
            Err(Error::OffsetMismatch { .. })
        ));
        // instant-preserving: the stale offset shifts the local time
        assert_eq!(stale.to_zoned().unwrap().datetime().hour(), 13);
    }

    #[test]
    fn fixed_offset() {
        let o: OffsetDateTime =
            round_trip("w.OffsetDateTime(2024, 2, 29, 3, 4, 5, offset=w.TimeDelta(seconds=-3723))");
        let jz = Zoned::try_from(o).unwrap();
        assert_eq!(jz.offset().seconds(), -3723);
        assert_eq!(
            jz.time_zone(),
            &TimeZone::fixed(jiff::tz::Offset::from_seconds(-3723).unwrap())
        );
        assert_eq!(OffsetDateTime::try_from(&jz).unwrap(), o);
        assert!(matches!(
            ZonedDateTime::try_from(&jz),
            Err(Error::MissingTimeZoneId)
        ));
    }
}
