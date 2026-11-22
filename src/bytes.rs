//! The binary format of `to_bytes()` and `from_bytes()`, for the backend
//! without a C API: the same fields as the records, little-endian, no
//! padding. The values come from whenever itself, so only the length is
//! checked here; whenever validates what it reads.

use crate::records::*;
use pyo3::{PyResult, exceptions::PyValueError};

fn malformed(type_name: &str) -> pyo3::PyErr {
    PyValueError::new_err(format!("malformed {type_name} bytes from whenever"))
}

/// The payload as an array of the length that the type has.
fn exact<'a, const N: usize>(b: &'a [u8], type_name: &str) -> PyResult<&'a [u8; N]> {
    b.try_into().map_err(|_| malformed(type_name))
}

pub(crate) fn encode_date(d: Date) -> [u8; 4] {
    let [y0, y1] = d.year.to_le_bytes();
    [y0, y1, d.month, d.day]
}

fn date(b: [u8; 4]) -> Date {
    let [y0, y1, month, day] = b;
    Date {
        year: u16::from_le_bytes([y0, y1]),
        month,
        day,
    }
}

pub(crate) fn decode_date(b: &[u8]) -> PyResult<Date> {
    exact(b, "Date").map(|b| date(*b))
}

pub(crate) fn encode_time(t: Time) -> [u8; 7] {
    let [n0, n1, n2, n3] = t.nanosecond.to_le_bytes();
    [t.hour, t.minute, t.second, n0, n1, n2, n3]
}

fn time(b: [u8; 7]) -> Time {
    let [hour, minute, second, nanosecond @ ..] = b;
    Time {
        hour,
        minute,
        second,
        nanosecond: u32::from_le_bytes(nanosecond),
    }
}

pub(crate) fn decode_time(b: &[u8]) -> PyResult<Time> {
    exact(b, "Time").map(|b| time(*b))
}

pub(crate) fn encode_plain(p: PlainDateTime) -> [u8; 11] {
    let mut out = [0; 11];
    out[..4].copy_from_slice(&encode_date(p.date));
    out[4..].copy_from_slice(&encode_time(p.time));
    out
}

fn plain(b: [u8; 11]) -> PlainDateTime {
    let [d0, d1, d2, d3, t @ ..] = b;
    PlainDateTime {
        date: date([d0, d1, d2, d3]),
        time: time(t),
    }
}

pub(crate) fn decode_plain(b: &[u8]) -> PyResult<PlainDateTime> {
    exact(b, "PlainDateTime").map(|b| plain(*b))
}

fn encode_second_nanos(second: i64, nanosecond: u32) -> [u8; 12] {
    let mut out = [0; 12];
    out[..8].copy_from_slice(&second.to_le_bytes());
    out[8..].copy_from_slice(&nanosecond.to_le_bytes());
    out
}

fn second_nanos(b: [u8; 12]) -> (i64, u32) {
    let [second @ .., n0, n1, n2, n3] = b;
    (
        i64::from_le_bytes(second),
        u32::from_le_bytes([n0, n1, n2, n3]),
    )
}

pub(crate) fn encode_instant(i: Instant) -> [u8; 12] {
    encode_second_nanos(i.epoch_second, i.nanosecond)
}

pub(crate) fn decode_instant(b: &[u8]) -> PyResult<Instant> {
    let (epoch_second, nanosecond) = second_nanos(*exact(b, "Instant")?);
    Ok(Instant {
        epoch_second,
        nanosecond,
    })
}

pub(crate) fn encode_time_delta(d: TimeDelta) -> [u8; 12] {
    encode_second_nanos(d.second, d.nanosecond)
}

pub(crate) fn decode_time_delta(b: &[u8]) -> PyResult<TimeDelta> {
    let (second, nanosecond) = second_nanos(*exact(b, "TimeDelta")?);
    Ok(TimeDelta { second, nanosecond })
}

pub(crate) fn encode_offset(o: OffsetDateTime) -> [u8; 15] {
    let mut out = [0; 15];
    out[..11].copy_from_slice(&encode_plain(o.local));
    out[11..].copy_from_slice(&o.offset_second.to_le_bytes());
    out
}

fn offset(b: [u8; 15]) -> OffsetDateTime {
    let [local @ .., o0, o1, o2, o3] = b;
    OffsetDateTime {
        local: plain(local),
        offset_second: i32::from_le_bytes([o0, o1, o2, o3]),
    }
}

pub(crate) fn decode_offset(b: &[u8]) -> PyResult<OffsetDateTime> {
    exact(b, "OffsetDateTime").map(|b| offset(*b))
}

pub(crate) fn encode_zoned(z: &ZonedDateTime, tz_id: &str) -> Vec<u8> {
    let mut out = Vec::with_capacity(15 + tz_id.len());
    out.extend_from_slice(&encode_offset(z.odt));
    out.extend_from_slice(tz_id.as_bytes());
    out
}

/// The local fields and offset, and the time zone ID borrowed from `b`.
pub(crate) fn decode_zoned(b: &[u8]) -> PyResult<(OffsetDateTime, &str)> {
    let (head, tz_id) = b
        .split_first_chunk::<15>()
        .ok_or_else(|| malformed("ZonedDateTime"))?;
    let tz_id = str::from_utf8(tz_id).map_err(|_| malformed("ZonedDateTime"))?;
    Ok((offset(*head), tz_id))
}
