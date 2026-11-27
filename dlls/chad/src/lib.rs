#![allow(non_snake_case, clippy::missing_safety_doc)]

#[path = "../../bridge.rs"]
mod bridge;

use std::ffi::c_void;
use std::sync::OnceLock;

const E_FAIL: i32 = 0x8000_4005_u32 as i32;

#[unsafe(no_mangle)]
pub unsafe extern "system" fn XDspConnectWithMaximumStreamLimit(
    base: *mut c_void,
    base_length: u32,
    ir_seconds: u32,
    max_streams: u32,
    max_channels: u32,
    out: *mut *mut c_void,
) -> i32 {
    type Target =
        unsafe extern "system" fn(*mut c_void, u32, u32, u32, u32, *mut *mut c_void) -> i32;
    static FN: OnceLock<usize> = OnceLock::new();
    let Some(addr) = bridge::cached(&FN, c"chad.dll", c"XDspConnectWithMaximumStreamLimit") else {
        return E_FAIL;
    };
    // SAFETY: resolved export matches `Target` ABI and arguments come from caller.
    unsafe {
        std::mem::transmute::<usize, Target>(addr)(
            base,
            base_length,
            ir_seconds,
            max_streams,
            max_channels,
            out,
        )
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "system" fn XDspDisconnect(dsp: *mut c_void) -> i32 {
    type Target = unsafe extern "system" fn(*mut c_void) -> i32;
    static FN: OnceLock<usize> = OnceLock::new();
    let Some(addr) = bridge::cached(&FN, c"chad.dll", c"XDspDisconnect") else {
        return E_FAIL;
    };
    // SAFETY: resolved export matches `Target` ABI and arguments come from caller.
    unsafe { std::mem::transmute::<usize, Target>(addr)(dsp) }
}

#[unsafe(no_mangle)]
pub unsafe extern "system" fn XDspActivate(
    dsp: *mut c_void,
    desc: *const c_void,
    status: *mut *const c_void,
    stream: *mut *mut c_void,
) -> i32 {
    type Target = unsafe extern "system" fn(
        *mut c_void,
        *const c_void,
        *mut *const c_void,
        *mut *mut c_void,
    ) -> i32;
    static FN: OnceLock<usize> = OnceLock::new();
    let Some(addr) = bridge::cached(&FN, c"chad.dll", c"XDspActivate") else {
        return E_FAIL;
    };
    // SAFETY: resolved export matches `Target` ABI and arguments come from caller.
    unsafe { std::mem::transmute::<usize, Target>(addr)(dsp, desc, status, stream) }
}

#[unsafe(no_mangle)]
pub unsafe extern "system" fn XDspDeactivate(stream: *mut c_void) -> i32 {
    type Target = unsafe extern "system" fn(*mut c_void) -> i32;
    static FN: OnceLock<usize> = OnceLock::new();
    let Some(addr) = bridge::cached(&FN, c"chad.dll", c"XDspDeactivate") else {
        return E_FAIL;
    };
    // SAFETY: resolved export matches `Target` ABI and arguments come from caller.
    unsafe { std::mem::transmute::<usize, Target>(addr)(stream) }
}

#[unsafe(no_mangle)]
pub unsafe extern "system" fn XDspSubmitCommand(
    stream: *mut c_void,
    command: *const c_void,
    ticket: *mut u32,
) -> i32 {
    type Target = unsafe extern "system" fn(*mut c_void, *const c_void, *mut u32) -> i32;
    static FN: OnceLock<usize> = OnceLock::new();
    let Some(addr) = bridge::cached(&FN, c"chad.dll", c"XDspSubmitCommand") else {
        return E_FAIL;
    };
    // SAFETY: resolved export matches `Target` ABI and arguments come from caller.
    unsafe { std::mem::transmute::<usize, Target>(addr)(stream, command, ticket) }
}
