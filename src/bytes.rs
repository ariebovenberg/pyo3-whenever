//! The binary format of `to_bytes()` and `from_bytes()`, for the backend
//! without a C API: the same fields as the records, little-endian, no
//! padding. The values come from whenever itself, so only the length is
//! checked here; whenever validates what it reads.

use crate::records::*;
use pyo3::{PyResult, exceptions::PyValueError};

fn malformed(type_name: &str) -> pyo3::PyErr {
    PyValueError::new_err(format!("malformed {type_name} bytes from whenever"))
}

fn u16_at(b: &[u8], i: usize) -> u16 {
    u16::from_le_bytes([b[i], b[i + 1]])
}

fn u32_at(b: &[u8], i: usize) -> u32 {
    u32::from_le_bytes([b[i], b[i + 1], b[i + 2], b[i + 3]])
}

fn i64_at(b: &[u8], i: usize) -> i64 {
    i64::from_le_bytes(b[i..i + 8].try_into().unwrap())
}

pub(crate) fn encode_date(d: Date) -> [u8; 4] {
    let [y0, y1] = d.year.to_le_bytes();
    [y0, y1, d.month, d.day]
}

pub(crate) fn decode_date(b: &[u8]) -> PyResult<Date> {
    if b.len() != 4 {
        return Err(malformed("Date"));
    }
    Ok(Date {
        year: u16_at(b, 0),
        month: b[2],
        day: b[3],
    })
}

pub(crate) fn encode_time(t: Time) -> [u8; 7] {
    let [n0, n1, n2, n3] = t.nanosecond.to_le_bytes();
    [t.hour, t.minute, t.second, n0, n1, n2, n3]
}

pub(crate) fn decode_time(b: &[u8]) -> PyResult<Time> {
    if b.len() != 7 {
        return Err(malformed("Time"));
    }
    Ok(Time {
        hour: b[0],
        minute: b[1],
        second: b[2],
        nanosecond: u32_at(b, 3),
    })
}

pub(crate) fn encode_plain(p: PlainDateTime) -> [u8; 11] {
    let mut out = [0; 11];
    out[..4].copy_from_slice(&encode_date(p.date));
    out[4..].copy_from_slice(&encode_time(p.time));
    out
}

pub(crate) fn decode_plain(b: &[u8]) -> PyResult<PlainDateTime> {
    if b.len() != 11 {
        return Err(malformed("PlainDateTime"));
    }
    Ok(PlainDateTime {
        date: decode_date(&b[..4])?,
        time: decode_time(&b[4..])?,
    })
}

fn encode_second_nanos(second: i64, nanosecond: u32) -> [u8; 12] {
    let mut out = [0; 12];
    out[..8].copy_from_slice(&second.to_le_bytes());
    out[8..].copy_from_slice(&nanosecond.to_le_bytes());
    out
}

pub(crate) fn encode_instant(i: Instant) -> [u8; 12] {
    encode_second_nanos(i.epoch_second, i.nanosecond)
}

pub(crate) fn decode_instant(b: &[u8]) -> PyResult<Instant> {
    if b.len() != 12 {
        return Err(malformed("Instant"));
    }
    Ok(Instant {
        epoch_second: i64_at(b, 0),
        nanosecond: u32_at(b, 8),
    })
}

pub(crate) fn encode_time_delta(d: TimeDelta) -> [u8; 12] {
    encode_second_nanos(d.second, d.nanosecond)
}

pub(crate) fn decode_time_delta(b: &[u8]) -> PyResult<TimeDelta> {
    if b.len() != 12 {
        return Err(malformed("TimeDelta"));
    }
    Ok(TimeDelta {
        second: i64_at(b, 0),
        nanosecond: u32_at(b, 8),
    })
}

pub(crate) fn encode_offset(o: OffsetDateTime) -> [u8; 15] {
    let mut out = [0; 15];
    out[..11].copy_from_slice(&encode_plain(o.local));
    out[11..].copy_from_slice(&o.offset_second.to_le_bytes());
    out
}

pub(crate) fn decode_offset(b: &[u8]) -> PyResult<OffsetDateTime> {
    if b.len() != 15 {
        return Err(malformed("OffsetDateTime"));
    }
    Ok(OffsetDateTime {
        local: decode_plain(&b[..11])?,
        offset_second: u32_at(b, 11) as i32,
    })
}

pub(crate) fn encode_zoned(z: &ZonedDateTime, tz_id: &str) -> Vec<u8> {
    let mut out = Vec::with_capacity(15 + tz_id.len());
    out.extend_from_slice(&encode_offset(z.odt));
    out.extend_from_slice(tz_id.as_bytes());
    out
}

pub(crate) fn decode_zoned(b: &[u8]) -> PyResult<ZonedDateTime> {
    let (head, tz_id) = b
        .split_at_checked(15)
        .ok_or_else(|| malformed("ZonedDateTime"))?;
    let tz_id = str::from_utf8(tz_id).map_err(|_| malformed("ZonedDateTime"))?;
    Ok(ZonedDateTime {
        odt: decode_offset(head)?,
        tz_id: Some(tz_id.to_owned()),
    })
}
