//! Conversions between the records and jiff's types.
//!
//! jiff's ranges are wider than whenever's in some places and narrower in
//! one, so most conversions are `TryFrom`:
//!
//! - jiff's civil types reach year -9999; whenever starts at year 1.
//! - jiff's `Timestamp::MAX` is about 26 hours before whenever's
//!   `Instant.MAX`, so the last day of year 9999 fits one but not the other.
//! - jiff offsets reach ±25:59:59; whenever's stop at ±23:59:59.
//!
//! A record is validated only when whenever builds an object from it, so
//! one built by hand with a field out of range fails on the way to jiff
//! as well, with [`Error::Jiff`] or [`Error::OutOfRange`].
//!
//! A [`ZonedDateTime`] becomes a [`Zoned`] with the same instant, placed in
//! the time zone with jiff's time zone data. When the two databases
//! disagree, the local time differs from whenever's;
//! [`ZonedDateTime::to_zoned_checked`] turns that into an error.

use crate::records::*;
use core::fmt;
use jiff::{
    SignedDuration, Timestamp, Zoned,
    civil::{self, DateTime},
    tz::{Offset, TimeZone},
};

/// Why a record and a jiff value do not convert.
#[derive(Debug)]
pub enum Error {
    /// The value is outside the target type's range; the string says which
    /// field.
    OutOfRange(&'static str),
    /// A `Zoned` whose time zone has no IANA name, or a `ZonedDateTime`
    /// without a time zone ID.
    MissingTimeZoneId,
    /// jiff's time zone data gives the instant a different offset than the
    /// record stores: the two databases disagree.
    OffsetMismatch {
        tz_id: String,
        stored: i32,
        computed: i32,
    },
    Jiff(jiff::Error),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::OutOfRange(what) => write!(f, "{what} out of range"),
            Self::MissingTimeZoneId => f.write_str("no time zone ID"),
            Self::OffsetMismatch {
                tz_id,
                stored,
                computed,
            } => write!(
                f,
                "stored offset {stored}s for {tz_id} but the time zone data gives {computed}s"
            ),
            Self::Jiff(e) => e.fmt(f),
        }
    }
}

impl std::error::Error for Error {}

impl From<jiff::Error> for Error {
    fn from(e: jiff::Error) -> Self {
        Self::Jiff(e)
    }
}

impl From<Error> for pyo3::PyErr {
    fn from(e: Error) -> Self {
        pyo3::exceptions::PyValueError::new_err(e.to_string())
    }
}

const MIN_EPOCH_SECOND: i64 = -62_135_596_800;
const MAX_DELTA_SECOND: i64 = 9_999 * 366 * 24 * 3_600;

impl TryFrom<Date> for civil::Date {
    type Error = Error;

    fn try_from(d: Date) -> Result<Self, Error> {
        // The other fields wrap into values jiff rejects; a year past i16
        // would wrap into one it accepts.
        let year = i16::try_from(d.year).map_err(|_| Error::OutOfRange("year"))?;
        Ok(civil::Date::new(year, d.month as i8, d.day as i8)?)
    }
}

impl TryFrom<civil::Date> for Date {
    type Error = Error;

    fn try_from(d: civil::Date) -> Result<Self, Error> {
        Ok(Self {
            year: u16::try_from(d.year())
                .ok()
                .filter(|y| *y >= 1)
                .ok_or(Error::OutOfRange("year"))?,
            month: d.month() as u8,
            day: d.day() as u8,
        })
    }
}

impl TryFrom<Time> for civil::Time {
    type Error = Error;

    fn try_from(t: Time) -> Result<Self, Error> {
        Ok(civil::Time::new(
            t.hour as i8,
            t.minute as i8,
            t.second as i8,
            t.nanosecond as i32,
        )?)
    }
}

impl From<civil::Time> for Time {
    fn from(t: civil::Time) -> Self {
        Self {
            hour: t.hour() as u8,
            minute: t.minute() as u8,
            second: t.second() as u8,
            nanosecond: t.subsec_nanosecond() as u32,
        }
    }
}

impl TryFrom<PlainDateTime> for DateTime {
    type Error = Error;

    fn try_from(p: PlainDateTime) -> Result<Self, Error> {
        Ok(DateTime::from_parts(p.date.try_into()?, p.time.try_into()?))
    }
}

impl TryFrom<DateTime> for PlainDateTime {
    type Error = Error;

    fn try_from(dt: DateTime) -> Result<Self, Error> {
        Ok(Self {
            date: dt.date().try_into()?,
            time: dt.time().into(),
        })
    }
}

impl TryFrom<Instant> for Timestamp {
    type Error = Error;

    fn try_from(i: Instant) -> Result<Self, Error> {
        Ok(Timestamp::new(i.epoch_second, i.nanosecond as i32)?)
    }
}

impl TryFrom<Timestamp> for Instant {
    type Error = Error;

    fn try_from(ts: Timestamp) -> Result<Self, Error> {
        let (epoch_second, nanosecond) = normalize(ts.as_second(), ts.subsec_nanosecond());
        if epoch_second < MIN_EPOCH_SECOND {
            return Err(Error::OutOfRange("timestamp"));
        }
        Ok(Self {
            epoch_second,
            nanosecond,
        })
    }
}

/// Seconds and a nanosecond of the same sign, as jiff stores them, to
/// seconds and a never-negative nanosecond, as whenever does.
fn normalize(second: i64, nanosecond: i32) -> (i64, u32) {
    if nanosecond < 0 {
        (second - 1, (nanosecond + 1_000_000_000) as u32)
    } else {
        (second, nanosecond as u32)
    }
}

impl TryFrom<TimeDelta> for SignedDuration {
    type Error = Error;

    fn try_from(d: TimeDelta) -> Result<Self, Error> {
        if d.nanosecond > 999_999_999 {
            return Err(Error::OutOfRange("nanosecond"));
        }
        Ok(SignedDuration::new(d.second, d.nanosecond as i32))
    }
}

impl TryFrom<SignedDuration> for TimeDelta {
    type Error = Error;

    fn try_from(d: SignedDuration) -> Result<Self, Error> {
        let (second, nanosecond) = normalize(d.as_secs(), d.subsec_nanos());
        if !(-MAX_DELTA_SECOND..=MAX_DELTA_SECOND).contains(&second)
            || (second == MAX_DELTA_SECOND && nanosecond > 0)
        {
            return Err(Error::OutOfRange("duration"));
        }
        Ok(Self { second, nanosecond })
    }
}

fn offset(second: i32) -> Result<Offset, Error> {
    Ok(Offset::from_seconds(second)?)
}

impl TryFrom<OffsetDateTime> for Zoned {
    type Error = Error;

    fn try_from(o: OffsetDateTime) -> Result<Self, Error> {
        Ok(DateTime::try_from(o.local)?.to_zoned(TimeZone::fixed(offset(o.offset_second)?))?)
    }
}

impl TryFrom<&Zoned> for OffsetDateTime {
    type Error = Error;

    /// The local fields and offset of any `Zoned`; its time zone is dropped.
    fn try_from(z: &Zoned) -> Result<Self, Error> {
        let offset_second = z.offset().seconds();
        if offset_second.abs() > 86_399 {
            return Err(Error::OutOfRange("offset"));
        }
        Ok(Self {
            local: z.datetime().try_into()?,
            offset_second,
        })
    }
}

impl ZonedDateTime {
    /// The instant in jiff's view of the time zone. The local fields may
    /// differ from the record's when jiff's and whenever's time zone data
    /// disagree; see [`Self::to_zoned_checked`].
    pub fn to_zoned(&self) -> Result<Zoned, Error> {
        let tz_id = self.tz_id.as_deref().ok_or(Error::MissingTimeZoneId)?;
        let tz = TimeZone::get(tz_id)?;
        Ok(Timestamp::try_from(self.instant())?.to_zoned(tz))
    }

    /// [`Self::to_zoned`], failing with [`Error::OffsetMismatch`] when
    /// jiff's time zone data gives the instant another offset than the
    /// record stores.
    pub fn to_zoned_checked(&self) -> Result<Zoned, Error> {
        let zoned = self.to_zoned()?;
        let computed = zoned.offset().seconds();
        if computed != self.odt.offset_second {
            return Err(Error::OffsetMismatch {
                tz_id: self.tz_id.clone().unwrap_or_default(),
                stored: self.odt.offset_second,
                computed,
            });
        }
        Ok(zoned)
    }
}

impl TryFrom<&ZonedDateTime> for Zoned {
    type Error = Error;

    fn try_from(z: &ZonedDateTime) -> Result<Self, Error> {
        z.to_zoned()
    }
}

impl TryFrom<&Zoned> for ZonedDateTime {
    type Error = Error;

    /// Requires an IANA time zone; a fixed-offset or POSIX zone has none,
    /// and converts to [`OffsetDateTime`] instead.
    fn try_from(z: &Zoned) -> Result<Self, Error> {
        let tz_id = z.time_zone().iana_name().ok_or(Error::MissingTimeZoneId)?;
        Ok(Self {
            odt: z.try_into()?,
            tz_id: Some(tz_id.to_owned()),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn instant_range() {
        let max = Instant {
            epoch_second: 253_402_300_799,
            nanosecond: 999_999_999,
        };
        assert!(matches!(Timestamp::try_from(max), Err(Error::Jiff(_))));
        let ts = Timestamp::new(-1, -1).unwrap();
        assert_eq!(
            Instant::try_from(ts).unwrap(),
            Instant {
                epoch_second: -2,
                nanosecond: 999_999_999
            }
        );
        assert!(matches!(
            Instant::try_from(Timestamp::MIN),
            Err(Error::OutOfRange("timestamp"))
        ));
    }

    #[test]
    fn duration_sign() {
        let d = TimeDelta {
            second: -1,
            nanosecond: 999_999_999,
        };
        let sd = SignedDuration::try_from(d).unwrap();
        assert_eq!(sd, SignedDuration::new(0, -1));
        assert_eq!(TimeDelta::try_from(sd).unwrap(), d);
        assert!(TimeDelta::try_from(SignedDuration::MAX).is_err());
    }

    #[test]
    fn hand_built_record_out_of_range() {
        let date = Date {
            year: 2024,
            month: 13,
            day: 1,
        };
        assert!(matches!(civil::Date::try_from(date), Err(Error::Jiff(_))));
        let date = Date {
            year: 55_537,
            month: 1,
            day: 1,
        };
        assert!(matches!(
            civil::Date::try_from(date),
            Err(Error::OutOfRange("year"))
        ));
        let time = Time {
            hour: 24,
            minute: 0,
            second: 0,
            nanosecond: 0,
        };
        assert!(matches!(civil::Time::try_from(time), Err(Error::Jiff(_))));
        let delta = TimeDelta {
            second: 0,
            nanosecond: 1_000_000_000,
        };
        assert!(matches!(
            SignedDuration::try_from(delta),
            Err(Error::OutOfRange("nanosecond"))
        ));
    }

    #[test]
    fn offset_range() {
        let z = DateTime::constant(2024, 2, 29, 3, 4, 5, 0)
            .to_zoned(TimeZone::fixed(Offset::from_seconds(25 * 3_600).unwrap()))
            .unwrap();
        assert!(matches!(
            OffsetDateTime::try_from(&z),
            Err(Error::OutOfRange("offset"))
        ));
        assert!(matches!(
            ZonedDateTime::try_from(&z),
            Err(Error::MissingTimeZoneId)
        ));
    }
}
