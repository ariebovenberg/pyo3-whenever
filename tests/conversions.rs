//! Round trips against the whenever installed next to this crate, on both
//! backends: `PYO3_WHENEVER_TEST_PURE=1` selects the pure-Python one.

use pyo3::{prelude::*, types::PyDict};
use pyo3_whenever::{records::*, uses_c_api};
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
    globals
        .set_item("dt", py.import("datetime").unwrap())
        .unwrap();
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
        // The only test that changes the system time zone: a second one
        // would race with it.
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

        #[cfg(feature = "jiff-02")]
        {
            let err = obj
                .extract::<pyo3_whenever::Whenever<jiff::Zoned>>()
                .unwrap_err();
            assert!(err.is_instance_of::<pyo3::exceptions::PyValueError>(py));
            assert!(err.to_string().contains("no time zone ID"), "{err}");
        }
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
            .to_pyobject_with(py, OffsetMismatch::KeepInstant, Disambiguation::Compatible)
            .unwrap();
        assert_eq!(kept.getattr("hour").unwrap().extract::<u8>().unwrap(), 13);
        let kept = stale
            .to_pyobject_with(py, OffsetMismatch::KeepLocal, Disambiguation::Raise)
            .unwrap();
        assert_eq!(kept.getattr("hour").unwrap().extract::<u8>().unwrap(), 12);
    });
}

#[test]
fn instant_to_tz() {
    attach(|py| {
        let instant = Instant {
            epoch_second: 1_709_175_845,
            nanosecond: 7,
        };
        let z = instant.to_tz(py, "Europe/Amsterdam").unwrap();
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
        let err = instant.to_tz(py, "Nope/Zone").unwrap_err();
        assert!(err.is_instance(py, &whenever(py).getattr("TimeZoneNotFoundError").unwrap()));
    });
}

#[test]
fn itemized_delta() {
    let d = round_trip::<ItemizedDelta>("w.ItemizedDelta(days=-2, hours=0, nanoseconds=-5)");
    assert_eq!(
        d,
        ItemizedDelta {
            days: Some(-2),
            hours: Some(0),
            seconds: Some(0),
            nanoseconds: Some(-5),
            ..ItemizedDelta::default()
        }
    );
    assert_eq!(
        round_trip::<ItemizedDelta>("w.ItemizedDelta()"),
        ItemizedDelta::default()
    );
    let d = round_trip::<ItemizedDateDelta>("w.ItemizedDateDelta(years=1, weeks=0)");
    assert_eq!(
        d,
        ItemizedDateDelta {
            years: Some(1),
            weeks: Some(0),
            ..ItemizedDateDelta::default()
        }
    );
    round_trip::<ItemizedDateDelta>("w.ItemizedDateDelta()");
    attach(|py| {
        let err = eval(py, "w.ItemizedDateDelta(days=1)")
            .extract::<ItemizedDelta>()
            .unwrap_err();
        assert!(err.is_instance_of::<pyo3::exceptions::PyTypeError>(py));
        // whenever validates the record.
        let mixed = ItemizedDelta {
            days: Some(1),
            hours: Some(-1),
            ..ItemizedDelta::default()
        };
        let err = mixed.into_pyobject(py).unwrap_err();
        assert!(err.is_instance_of::<pyo3::exceptions::PyValueError>(py));
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
    use pyo3_whenever::Error;

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

#[cfg(feature = "jiff-02")]
mod wrappers {
    use super::*;
    use jiff::{
        SignedDuration, Span, Timestamp, Zoned, civil,
        tz::{Offset, TimeZone},
    };
    use pyo3::exceptions::{PyTypeError, PyValueError};
    use pyo3_whenever::{Convert, Whenever, WheneverOrDatetime};

    fn strict_eq(a: &Bound<'_, PyAny>, b: &Bound<'_, PyAny>) -> bool {
        a.get_type().is(b.get_type())
            && a.eq(b).unwrap()
            && (!a.hasattr("strict_eq").unwrap()
                || a.call_method1("strict_eq", (b,))
                    .unwrap()
                    .is_truthy()
                    .unwrap())
    }

    /// Extract through `Whenever<T>`, check that the value comes back as
    /// an identical object of the same class, and return it.
    fn round_trip<T: Convert + Clone>(code: &str) -> T {
        attach(|py| {
            let obj = eval(py, code);
            let Whenever(value) = obj.extract::<Whenever<T>>().unwrap();
            let back = Whenever(value.clone()).into_pyobject(py).unwrap();
            assert!(strict_eq(&obj, &back), "{code}: {back}");
            value
        })
    }

    fn extract_err<T>(code: &str) -> (bool, String)
    where
        T: for<'a, 'py> FromPyObject<'a, 'py, Error = PyErr>,
    {
        attach(|py| {
            let Err(e) = eval(py, code).extract::<T>() else {
                panic!("{code} extracted");
            };
            assert!(e.is_instance_of::<PyTypeError>(py) || e.is_instance_of::<PyValueError>(py));
            (e.is_instance_of::<PyTypeError>(py), e.to_string())
        })
    }

    #[test]
    fn scalars() {
        assert_eq!(
            round_trip::<civil::Date>("w.Date(2024, 2, 29)"),
            civil::date(2024, 2, 29)
        );
        assert_eq!(
            round_trip::<civil::Time>("w.Time(3, 4, 5, nanosecond=600_700_800)"),
            civil::time(3, 4, 5, 600_700_800)
        );
        assert_eq!(
            round_trip::<civil::DateTime>("w.PlainDateTime(2024, 2, 29, 3, 4, 5, nanosecond=7)"),
            civil::datetime(2024, 2, 29, 3, 4, 5, 7)
        );
        assert_eq!(
            round_trip::<Timestamp>("w.Instant.from_utc(2024, 2, 29, 3, 4, 5, nanosecond=7)"),
            Timestamp::new(1_709_175_845, 7).unwrap()
        );
        assert_eq!(
            round_trip::<SignedDuration>("w.TimeDelta(seconds=-12_345, nanoseconds=678_901_234)"),
            SignedDuration::new(-12_344, -321_098_766)
        );
        round_trip::<civil::Date>("w.Date.MIN");
        round_trip::<civil::Date>("w.Date.MAX");
        round_trip::<civil::Time>("w.Time.MAX");
        round_trip::<Timestamp>("w.Instant.MIN");
        round_trip::<SignedDuration>("w.TimeDelta.MIN");
        round_trip::<SignedDuration>("w.TimeDelta.MAX");
    }

    #[test]
    fn zoned() {
        let z = round_trip::<Zoned>(
            "w.ZonedDateTime(2024, 7, 1, 12, nanosecond=7, tz='Europe/Amsterdam')",
        );
        assert_eq!(z.time_zone().iana_name(), Some("Europe/Amsterdam"));
        assert_eq!(z.datetime(), civil::datetime(2024, 7, 1, 12, 0, 0, 7));
        // a repeated local time keeps its occurrence
        let z = round_trip::<Zoned>(
            "w.ZonedDateTime(2023, 10, 29, 2, 30, tz='Europe/Amsterdam', disambiguation='later')",
        );
        assert_eq!(z.offset().seconds(), 3_600);
    }

    #[test]
    fn offset_datetime_is_a_fixed_zoned() {
        let z = round_trip::<Zoned>(
            "w.OffsetDateTime(2024, 2, 29, 3, 4, 5, nanosecond=7, offset=w.TimeDelta(seconds=-3723))",
        );
        assert_eq!(
            z.time_zone(),
            &TimeZone::fixed(Offset::from_seconds(-3_723).unwrap())
        );
        assert_eq!(z.datetime(), civil::datetime(2024, 2, 29, 3, 4, 5, 7));
    }

    /// jiff has one value for UTC and for a fixed offset of zero.
    #[test]
    fn zero_offset_comes_back_in_utc() {
        attach(|py| {
            let obj = eval(
                py,
                "w.OffsetDateTime(2024, 2, 29, 3, 4, 5, offset=w.TimeDelta())",
            );
            let Whenever(z) = obj.extract::<Whenever<Zoned>>().unwrap();
            assert_eq!(z.time_zone(), &TimeZone::UTC);
            let back = Whenever(z).into_pyobject(py).unwrap();
            let expected = eval(py, "w.ZonedDateTime(2024, 2, 29, 3, 4, 5, tz='UTC')");
            assert!(strict_eq(&back, &expected), "{back}");
            round_trip::<Zoned>("w.ZonedDateTime(2024, 2, 29, 3, 4, 5, tz='UTC')");
        });
    }

    #[test]
    fn zoned_in_a_posix_time_zone() {
        attach(|py| {
            let posix = Timestamp::UNIX_EPOCH
                .to_zoned(TimeZone::posix("CET-1CEST,M3.5.0,M10.5.0/3").unwrap());
            let err = Whenever(posix).into_pyobject(py).unwrap_err();
            assert!(err.is_instance_of::<PyValueError>(py));
            assert!(err.to_string().contains("no IANA name"), "{err}");
        });
    }

    #[test]
    fn wrong_class() {
        let (type_error, msg) = extract_err::<Whenever<civil::Date>>("w.Time(1)");
        assert!(type_error);
        assert_eq!(msg, "TypeError: expected a whenever.Date, got Time");

        let (type_error, msg) = extract_err::<Whenever<Zoned>>("w.Instant.from_timestamp(5)");
        assert!(type_error);
        assert_eq!(
            msg,
            "TypeError: expected a whenever.ZonedDateTime or whenever.OffsetDateTime, got Instant"
        );

        // `Whenever` takes no `datetime` objects
        let (type_error, _) = extract_err::<Whenever<civil::Date>>("dt.date(2024, 2, 29)");
        assert!(type_error);
    }

    #[test]
    fn out_of_range() {
        let (type_error, msg) = extract_err::<Whenever<Timestamp>>("w.Instant.MAX");
        assert!(!type_error);
        assert!(
            msg.starts_with("ValueError: cannot convert this whenever.Instant: "),
            "{msg}"
        );

        attach(|py| {
            let err = Whenever(civil::date(0, 12, 31))
                .into_pyobject(py)
                .unwrap_err();
            assert!(err.is_instance_of::<PyValueError>(py));
            assert_eq!(
                err.to_string(),
                "ValueError: cannot convert to a whenever.Date: year out of range"
            );
            let far = Timestamp::UNIX_EPOCH
                .to_zoned(TimeZone::fixed(Offset::from_seconds(25 * 3_600).unwrap()));
            let err = Whenever(far).into_pyobject(py).unwrap_err();
            assert!(err.to_string().contains("offset out of range"), "{err}");
        });
    }

    #[test]
    fn span() {
        let s = round_trip::<Span>(
            "w.ItemizedDelta(years=1, months=2, weeks=3, days=4, hours=5, minutes=6, seconds=7, nanoseconds=8)",
        );
        assert_eq!(
            s.fieldwise(),
            Span::new()
                .years(1)
                .months(2)
                .weeks(3)
                .days(4)
                .hours(5)
                .minutes(6)
                .seconds(7)
                .nanoseconds(8)
        );
        let s = round_trip::<Span>("w.ItemizedDelta(days=-1, seconds=-3, nanoseconds=-5)");
        assert_eq!(
            s.fieldwise(),
            Span::new().days(-1).seconds(-3).nanoseconds(-5)
        );
        assert!(round_trip::<Span>("w.ItemizedDelta()").is_zero());

        attach(|py| {
            let back = |code: &str| {
                let Whenever(s) = eval(py, code).extract::<Whenever<Span>>().unwrap();
                Whenever(s).into_pyobject(py).unwrap()
            };
            // Zero components do not come back, and the class is always
            // `ItemizedDelta`.
            assert!(strict_eq(
                &back("w.ItemizedDelta(days=2, hours=0)"),
                &eval(py, "w.ItemizedDelta(days=2)")
            ));
            assert!(strict_eq(
                &back("w.ItemizedDateDelta(years=1, days=0)"),
                &eval(py, "w.ItemizedDelta(years=1)")
            ));
            assert!(strict_eq(
                &back("w.ItemizedDelta(seconds=0)"),
                &eval(py, "w.ItemizedDelta()")
            ));

            let ms = Span::new().milliseconds(1_500).microseconds(2);
            assert!(strict_eq(
                &Whenever(ms).into_pyobject(py).unwrap(),
                &eval(py, "w.ItemizedDelta(seconds=1, nanoseconds=500_002_000)")
            ));

            // jiff allows up to 19,998 years; whenever up to 9,999.
            let err = Whenever(Span::new().years(10_000))
                .into_pyobject(py)
                .unwrap_err();
            assert!(err.is_instance_of::<PyValueError>(py), "{err}");
        });

        let (type_error, msg) = extract_err::<Whenever<Span>>("w.TimeDelta(hours=1)");
        assert!(type_error);
        assert_eq!(
            msg,
            "TypeError: expected a whenever.ItemizedDelta or whenever.ItemizedDateDelta, got TimeDelta"
        );
        assert_eq!(
            lenient::<Span>("dt.timedelta(days=1, microseconds=5)").fieldwise(),
            Span::new().seconds(86_400).microseconds(5)
        );
        assert_eq!(
            lenient::<Span>("w.ItemizedDateDelta(weeks=2)").fieldwise(),
            Span::new().weeks(2)
        );
    }

    fn lenient<T: Convert>(code: &str) -> T {
        attach(|py| eval(py, code).extract::<WheneverOrDatetime<T>>().unwrap().0)
    }

    #[test]
    fn or_datetime_takes_whenever() {
        assert_eq!(
            lenient::<Timestamp>("w.Instant.from_utc(2024, 2, 29, 3, 4, 5, nanosecond=7)"),
            Timestamp::new(1_709_175_845, 7).unwrap()
        );
        assert_eq!(
            lenient::<Zoned>("w.OffsetDateTime(2024, 2, 29, 3, 4, 5, offset=w.TimeDelta(hours=2))")
                .offset()
                .seconds(),
            7_200
        );
    }

    #[test]
    fn or_datetime_takes_datetime() {
        assert_eq!(
            lenient::<civil::Date>("dt.date(2024, 2, 29)"),
            civil::date(2024, 2, 29)
        );
        assert_eq!(
            lenient::<civil::Time>("dt.time(3, 4, 5, 600_700)"),
            civil::time(3, 4, 5, 600_700_000)
        );
        assert_eq!(
            lenient::<civil::DateTime>("dt.datetime(2024, 2, 29, 3, 4, 5)"),
            civil::datetime(2024, 2, 29, 3, 4, 5, 0)
        );
        assert_eq!(
            lenient::<Timestamp>("dt.datetime(2024, 2, 29, 3, 4, 5, tzinfo=dt.timezone.utc)"),
            Timestamp::new(1_709_175_845, 0).unwrap()
        );
        assert_eq!(
            lenient::<SignedDuration>("dt.timedelta(seconds=-12_345, microseconds=678_901)"),
            SignedDuration::new(-12_344, -321_099_000)
        );
        assert_eq!(
            lenient::<Zoned>(
                "dt.datetime(2024, 7, 1, 12, tzinfo=dt.timezone(dt.timedelta(hours=2)))"
            )
            .offset()
            .seconds(),
            7_200
        );
        // PyO3's rule: a `datetime` is a `date`
        assert_eq!(
            lenient::<civil::Date>("dt.datetime(2024, 2, 29, 12, 30)"),
            civil::date(2024, 2, 29)
        );
    }

    #[test]
    fn or_datetime_errors() {
        let (type_error, msg) = extract_err::<WheneverOrDatetime<Timestamp>>("'2024-02-29'");
        assert!(type_error);
        assert_eq!(
            msg,
            "TypeError: expected a whenever.Instant or datetime.datetime, got str"
        );

        let (type_error, msg) = extract_err::<WheneverOrDatetime<Zoned>>("w.Date(2024, 2, 29)");
        assert!(type_error);
        assert_eq!(
            msg,
            "TypeError: expected a whenever.ZonedDateTime, whenever.OffsetDateTime, \
             or datetime.datetime, got Date"
        );

        // the right `datetime` class with the wrong content: PyO3's error
        let (_, msg) =
            extract_err::<WheneverOrDatetime<Timestamp>>("dt.datetime(2024, 2, 29, 3, 4, 5)");
        assert!(msg.contains("tzinfo"), "{msg}");

        // a whenever object out of range does not fall through to `datetime`
        let (type_error, msg) = extract_err::<WheneverOrDatetime<Timestamp>>("w.Instant.MAX");
        assert!(!type_error);
        assert!(msg.contains("whenever.Instant"), "{msg}");
    }

    #[pyfunction]
    fn shift(
        start: WheneverOrDatetime<Timestamp>,
        by: Option<Whenever<SignedDuration>>,
        more: Vec<Whenever<SignedDuration>>,
    ) -> Whenever<Timestamp> {
        let total = by
            .into_iter()
            .chain(more)
            .fold(SignedDuration::ZERO, |a, d| a + d.0);
        Whenever(start.0 + total)
    }

    #[test]
    fn in_a_signature() {
        attach(|py| {
            let globals = PyDict::new(py);
            globals.set_item("w", whenever(py)).unwrap();
            globals
                .set_item("dt", py.import("datetime").unwrap())
                .unwrap();
            globals
                .set_item("shift", wrap_pyfunction!(shift, py).unwrap())
                .unwrap();
            py.run(
                c"
start = w.Instant.from_utc(2024, 2, 29)
hour = w.TimeDelta(hours=1)
assert shift(start, hour, [hour, hour]) == start + 3 * hour
assert shift(start.to_stdlib(), None, []) == start
",
                Some(&globals),
                None,
            )
            .unwrap();
        });
    }
}
