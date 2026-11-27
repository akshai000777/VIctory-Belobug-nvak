#![allow(non_snake_case, clippy::missing_safety_doc)]

use std::ffi::{CStr, c_char};
#[cfg(windows)]
use std::ffi::c_void;

#[cfg(windows)]
use victory_hle::modules::xmem::{self, BasicInformation, WorkingSet};

/// Shared HLE export resolver used by `d3d12_xs.dll`, `chad.dll`, and
/// `xg_xs.dll` so all four DLLs share one `victory-hle` / `victory-gpu` instance.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn victory_hle_lookup_export(
    dll: *const c_char,
    name: *const c_char,
) -> usize {
    if dll.is_null() || name.is_null() {
        return 0;
    }
    #[cfg(windows)]
    victory_hle::ensure_standalone_host_init();
    // SAFETY: callers pass valid NUL-terminated C strings.
    let (dll, name) = unsafe { (CStr::from_ptr(dll), CStr::from_ptr(name)) };
    let (Ok(dll), Ok(name)) = (dll.to_str(), name.to_str()) else {
        return 0;
    };
    victory_hle::lookup_export(dll, name).unwrap_or(0)
}

#[cfg(windows)]
#[unsafe(no_mangle)]
pub extern "system" fn XMemAlloc(size: usize, attributes: u64) -> *mut c_void {
    victory_hle::ensure_standalone_host_init();
    xmem::XMemAlloc(size, attributes)
}

#[cfg(windows)]
#[unsafe(no_mangle)]
pub extern "system" fn XMemFree(address: *mut c_void, attributes: u64) {
    victory_hle::ensure_standalone_host_init();
    xmem::XMemFree(address, attributes);
}

#[cfg(windows)]
#[unsafe(no_mangle)]
pub extern "system" fn XMemAllocDefault(size: usize, attributes: u64) -> *mut c_void {
    victory_hle::ensure_standalone_host_init();
    xmem::XMemAllocDefault(size, attributes)
}

#[cfg(windows)]
#[unsafe(no_mangle)]
pub extern "system" fn XMemFreeDefault(address: *mut c_void, attributes: u64) {
    victory_hle::ensure_standalone_host_init();
    xmem::XMemFreeDefault(address, attributes);
}

#[cfg(windows)]
#[unsafe(no_mangle)]
pub extern "system" fn XMemIsToolsMemAvailable() -> i32 {
    victory_hle::ensure_standalone_host_init();
    xmem::XMemIsToolsMemAvailable()
}

#[cfg(windows)]
#[unsafe(no_mangle)]
pub extern "system" fn XMemAllocatePhysicalPages(
    flags: u64,
    count: *mut usize,
    pages: *mut usize,
) -> i32 {
    victory_hle::ensure_standalone_host_init();
    xmem::XMemAllocatePhysicalPages(flags, count, pages)
}

#[cfg(windows)]
#[unsafe(no_mangle)]
pub extern "system" fn XMemFreePhysicalPages(n: usize, pages: *const usize) -> i32 {
    victory_hle::ensure_standalone_host_init();
    xmem::XMemFreePhysicalPages(n, pages)
}

#[cfg(windows)]
#[unsafe(no_mangle)]
pub extern "system" fn XMemMapPhysicalPages(
    address: *mut c_void,
    n: usize,
    pages: *const usize,
) -> *mut c_void {
    victory_hle::ensure_standalone_host_init();
    xmem::XMemMapPhysicalPages(address, n, pages)
}

#[cfg(windows)]
#[unsafe(no_mangle)]
pub extern "system" fn XMemMapPhysicalPagesScatter(
    addresses: *const *mut c_void,
    n: usize,
    pages: *const usize,
) -> i32 {
    victory_hle::ensure_standalone_host_init();
    xmem::XMemMapPhysicalPagesScatter(addresses, n, pages)
}

#[cfg(windows)]
#[unsafe(no_mangle)]
pub extern "system" fn XMemVirtualAlloc(
    base: *mut c_void,
    size: usize,
    kind: u32,
    flags: u64,
    protect: u32,
) -> *mut c_void {
    victory_hle::ensure_standalone_host_init();
    xmem::XMemVirtualAlloc(base, size, kind, flags, protect)
}

#[cfg(windows)]
#[unsafe(no_mangle)]
pub extern "system" fn XMemVirtualQuery(
    process: *mut c_void,
    address: *const c_void,
    out: *mut BasicInformation,
) -> i32 {
    victory_hle::ensure_standalone_host_init();
    xmem::XMemVirtualQuery(process, address, out)
}

#[cfg(windows)]
#[unsafe(no_mangle)]
pub extern "system" fn XMemGetWorkingSetStatistics(which: u32, out: *mut WorkingSet) -> i32 {
    victory_hle::ensure_standalone_host_init();
    xmem::XMemGetWorkingSetStatistics(which, out)
}

#[cfg(windows)]
#[unsafe(no_mangle)]
pub extern "system" fn XMemTransferMemory(kind: u32, size: usize) -> i32 {
    victory_hle::ensure_standalone_host_init();
    xmem::XMemTransferMemory(kind, size)
}
