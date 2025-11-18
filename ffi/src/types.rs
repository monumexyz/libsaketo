use std::ffi::{c_char, CStr, CString};

#[repr(C)]
pub(crate) struct ResultWithMessage {
    success: bool,
    message: *const c_char,
}

impl ResultWithMessage {
    pub(crate) fn new(success: bool, message: &str) -> Self {
        let c_message = CString::new(message).unwrap();
        ResultWithMessage {
            success,
            message: c_message.into_raw(),
        }
    }
}

pub(crate) fn c_str_to_string(c_str: *const c_char) -> String {
    let c_str = unsafe {
        assert!(!c_str.is_null());
        CStr::from_ptr(c_str)
    };
    c_str.to_str().unwrap_or("").to_string()
}

#[unsafe(no_mangle)]
pub extern "C" fn get_block_height_from_unix_time(unix_time: i64) -> i64 {
    let time_diff = unix_time.saturating_sub(1635724948); // This number corresponds to Polyseed's earliest possible birthday
    let early_day_seconds = time_diff / 730; // A day earlier for every two years for safety
    let block_height = time_diff.saturating_sub(early_day_seconds) / 120;
    (2483380 + block_height).try_into().unwrap() // This number corresponds to Monero's block height at Polyseed's earliest possible birthday
}

#[unsafe(no_mangle)]
extern "C" fn free_c_string(s: *mut c_char) {
    if !s.is_null() {
        unsafe {
            let _ = CString::from_raw(s);
        }
    }
}