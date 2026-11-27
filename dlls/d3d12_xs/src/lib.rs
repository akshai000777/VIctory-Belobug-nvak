#![allow(non_snake_case, non_upper_case_globals, clippy::missing_safety_doc)]

#[path = "../../bridge.rs"]
mod bridge;

use std::ffi::c_void;
use std::sync::OnceLock;

const E_FAIL: i32 = 0x8000_4005_u32 as i32;

/// `ID3D12Device` interface GUID (`{189819f1-1db6-4b57-be54-1821339b85f7}`),
/// imported directly as data by Xbox titles.
#[unsafe(no_mangle)]
pub static IID_ID3D12Device: [u8; 16] = [
    0xf1, 0x19, 0x98, 0x18, 0xb6, 0x1d, 0x57, 0x4b, 0xbe, 0x54, 0x18, 0x21, 0x33, 0x9b, 0x85, 0xf7,
];

#[unsafe(no_mangle)]
pub unsafe extern "system" fn D3D12ConfigureVirtualMemoryX(config: *mut c_void) -> i32 {
    type Target = unsafe extern "system" fn(*mut c_void) -> i32;
    static FN: OnceLock<usize> = OnceLock::new();
    let Some(addr) = bridge::cached(&FN, c"d3d12_xs.dll", c"D3D12ConfigureVirtualMemoryX") else {
        return E_FAIL;
    };
    // SAFETY: resolved export matches `Target` ABI and arguments come from caller.
    unsafe { std::mem::transmute::<usize, Target>(addr)(config) }
}

#[unsafe(no_mangle)]
pub unsafe extern "system" fn D3D12XboxCreateDevice(
    adapter: *const c_void,
    parameters: *const u32,
    iid: *const [u8; 16],
    out: *mut *const c_void,
) -> i32 {
    type Target = unsafe extern "system" fn(
        *const c_void,
        *const u32,
        *const [u8; 16],
        *mut *const c_void,
    ) -> i32;
    static FN: OnceLock<usize> = OnceLock::new();
    let Some(addr) = bridge::cached(&FN, c"d3d12_xs.dll", c"D3D12XboxCreateDevice") else {
        return E_FAIL;
    };
    // SAFETY: resolved export matches `Target` ABI and arguments come from caller.
    unsafe { std::mem::transmute::<usize, Target>(addr)(adapter, parameters, iid, out) }
}

#[unsafe(no_mangle)]
pub unsafe extern "system" fn D3D12SerializeRootSignature(
    desc: *const c_void,
    version: u32,
    blob: *mut *mut c_void,
    error: *mut *mut c_void,
) -> i32 {
    type Target =
        unsafe extern "system" fn(*const c_void, u32, *mut *mut c_void, *mut *mut c_void) -> i32;
    static FN: OnceLock<usize> = OnceLock::new();
    let Some(addr) = bridge::cached(&FN, c"d3d12_xs.dll", c"D3D12SerializeRootSignature") else {
        return E_FAIL;
    };
    // SAFETY: resolved export matches `Target` ABI and arguments come from caller.
    unsafe { std::mem::transmute::<usize, Target>(addr)(desc, version, blob, error) }
}

#[unsafe(no_mangle)]
pub unsafe extern "system" fn D3D12SerializeVersionedRootSignature(
    desc: *const c_void,
    blob: *mut *mut c_void,
    error: *mut *mut c_void,
) -> i32 {
    type Target =
        unsafe extern "system" fn(*const c_void, *mut *mut c_void, *mut *mut c_void) -> i32;
    static FN: OnceLock<usize> = OnceLock::new();
    let Some(addr) = bridge::cached(&FN, c"d3d12_xs.dll", c"D3D12SerializeVersionedRootSignature")
    else {
        return E_FAIL;
    };
    // SAFETY: resolved export matches `Target` ABI and arguments come from caller.
    unsafe { std::mem::transmute::<usize, Target>(addr)(desc, blob, error) }
}

#[unsafe(no_mangle)]
pub unsafe extern "system" fn D3D12CreateRootSignatureDeserializer(
    blob: *const u8,
    size: usize,
    iid: *const [u8; 16],
    out: *mut *mut c_void,
) -> i32 {
    type Target =
        unsafe extern "system" fn(*const u8, usize, *const [u8; 16], *mut *mut c_void) -> i32;
    static FN: OnceLock<usize> = OnceLock::new();
    let Some(addr) = bridge::cached(&FN, c"d3d12_xs.dll", c"D3D12CreateRootSignatureDeserializer")
    else {
        return E_FAIL;
    };
    // SAFETY: resolved export matches `Target` ABI and arguments come from caller.
    unsafe { std::mem::transmute::<usize, Target>(addr)(blob, size, iid, out) }
}
