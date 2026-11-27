#![allow(non_snake_case, clippy::missing_safety_doc)]

#[path = "../../bridge.rs"]
mod bridge;

use std::ffi::c_void;
use std::sync::OnceLock;

const E_FAIL: i32 = 0x8000_4005_u32 as i32;

#[unsafe(no_mangle)]
pub unsafe extern "system" fn XGSuggestSwizzleMode(
    desc: *const c_void,
    flags: u32,
    out: *mut u32,
) -> i32 {
    type Target = unsafe extern "system" fn(*const c_void, u32, *mut u32) -> i32;
    static FN: OnceLock<usize> = OnceLock::new();
    let Some(addr) = bridge::cached(&FN, c"xg_xs.dll", c"XGSuggestSwizzleMode") else {
        return E_FAIL;
    };
    // SAFETY: resolved export matches `Target` ABI and arguments come from caller.
    unsafe { std::mem::transmute::<usize, Target>(addr)(desc, flags, out) }
}

#[unsafe(no_mangle)]
pub unsafe extern "system" fn XGCreateTextureComputer(
    desc: *const c_void,
    out: *mut *mut c_void,
) -> i32 {
    type Target = unsafe extern "system" fn(*const c_void, *mut *mut c_void) -> i32;
    static FN: OnceLock<usize> = OnceLock::new();
    let Some(addr) = bridge::cached(&FN, c"xg_xs.dll", c"XGCreateTextureComputer") else {
        return E_FAIL;
    };
    // SAFETY: resolved export matches `Target` ABI and arguments come from caller.
    unsafe { std::mem::transmute::<usize, Target>(addr)(desc, out) }
}
