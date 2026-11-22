//! `include/whenever.h` from the whenever repository, as Rust. Field for
//! field the same, including the order of the table; the header is the
//! contract and this file follows it.
#![allow(non_camel_case_types, dead_code)]

use core::{
    ffi::{CStr, c_char, c_int, c_void},
    ptr::NonNull,
};
use pyo3::ffi::{PyObject, PyTypeObject};

pub(crate) const CAPSULE_NAME: &CStr = c"whenever.C_API_v1";

#[repr(C)]
#[derive(Clone, Copy)]
pub(crate) struct Date {
    pub year: u16,
    pub month: u8,
    pub day: u8,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub(crate) struct Time {
    pub hour: u8,
    pub minute: u8,
    pub second: u8,
    pub nanosecond: u32,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub(crate) struct PlainDateTime {
    pub date: Date,
    pub time: Time,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub(crate) struct Instant {
    pub epoch_second: i64,
    pub nanosecond: u32,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub(crate) struct OffsetDateTime {
    pub local: PlainDateTime,
    pub offset_second: i32,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub(crate) struct ZonedDateTime {
    pub odt: OffsetDateTime,
    pub tz_id: *const c_char,
    pub tz_id_len: u8,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub(crate) struct TimeDelta {
    pub second: i64,
    pub nanosecond: u32,
}

/// 0: written. 1: not exactly this type, nothing written, no exception.
pub(crate) type Extract<R> = unsafe extern "C" fn(*const Table, *mut PyObject, *mut R) -> c_int;
/// A new reference, or null with an exception set.
pub(crate) type New<R> = unsafe extern "C" fn(*const Table, *const R) -> *mut PyObject;

#[repr(C)]
pub(crate) struct Table {
    pub magic: *const c_char,
    pub struct_size: i32,
    pub version_minor: i32,

    pub date_type: *mut PyTypeObject,
    pub time_type: *mut PyTypeObject,
    pub plain_datetime_type: *mut PyTypeObject,
    pub instant_type: *mut PyTypeObject,
    pub offset_datetime_type: *mut PyTypeObject,
    pub zoned_datetime_type: *mut PyTypeObject,
    pub time_delta_type: *mut PyTypeObject,

    pub date_extract: Extract<Date>,
    pub time_extract: Extract<Time>,
    pub plain_datetime_extract: Extract<PlainDateTime>,
    pub instant_extract: Extract<Instant>,
    pub offset_datetime_extract: Extract<OffsetDateTime>,
    pub zoned_datetime_extract: Extract<ZonedDateTime>,
    pub time_delta_extract: Extract<TimeDelta>,

    pub date_new: New<Date>,
    pub time_new: New<Time>,
    pub plain_datetime_new: New<PlainDateTime>,
    pub instant_new: New<Instant>,
    pub offset_datetime_new: New<OffsetDateTime>,
    pub time_delta_new: New<TimeDelta>,
    pub zoned_datetime_from_instant:
        unsafe extern "C" fn(*const Table, *const Instant, *const c_char, u8) -> *mut PyObject,
    pub zoned_datetime_new:
        unsafe extern "C" fn(*const Table, *const ZonedDateTime, c_int, c_int) -> *mut PyObject,
}

/// The fields that every build of the table starts with.
#[repr(C)]
struct Header {
    magic: *const c_char,
    struct_size: i32,
}

impl Table {
    /// The table behind the capsule's pointer, or `None` when it is not a
    /// table that reaches as far as this file's.
    ///
    /// # Safety
    ///
    /// `ptr` is the pointer of the `CAPSULE_NAME` capsule, and the caller
    /// keeps the module that owns the table alive for `'static`.
    pub(crate) unsafe fn from_capsule(ptr: NonNull<c_void>) -> Option<&'static Self> {
        // SAFETY: every build publishes at least the header.
        let header = unsafe { ptr.cast::<Header>().as_ref() };
        // SAFETY: `magic` is a static C string.
        let magic = unsafe { CStr::from_ptr(header.magic) };
        let complete = usize::try_from(header.struct_size).is_ok_and(|n| n >= size_of::<Self>());
        // SAFETY: the table is at least as large as `Self`, and it is
        // immutable once published.
        (magic == CAPSULE_NAME && complete).then(|| unsafe { ptr.cast::<Self>().as_ref() })
    }
}

// The table is immutable once published, and its pointers are owned by the
// whenever module, which outlives any use through this crate.
unsafe impl Sync for Table {}
unsafe impl Send for Table {}
