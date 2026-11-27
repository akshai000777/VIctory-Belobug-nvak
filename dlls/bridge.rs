use std::ffi::CStr;
use std::sync::OnceLock;

#[cfg(windows)]
mod imp {
    use std::ffi::{CStr, c_char, c_void};
    use std::sync::OnceLock;

    const GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS: u32 = 0x0000_0004;
    const GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT: u32 = 0x0000_0002;

    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetModuleHandleW(name: *const u16) -> *mut c_void;
        fn GetModuleHandleExW(flags: u32, addr: *const u16, out: *mut *mut c_void) -> i32;
        fn GetModuleFileNameW(module: *mut c_void, buf: *mut u16, size: u32) -> u32;
        fn LoadLibraryW(name: *const u16) -> *mut c_void;
        fn GetProcAddress(module: *mut c_void, name: *const c_char) -> *const c_void;
    }

    type LookupFn = unsafe extern "system" fn(*const c_char, *const c_char) -> usize;

    fn lookup_fn() -> Option<LookupFn> {
        static LOOKUP: OnceLock<usize> = OnceLock::new();
        if let Some(&addr) = LOOKUP.get() {
            // SAFETY: `victory_hle_lookup_export` has `LookupFn`'s exact ABI.
            return Some(unsafe { std::mem::transmute::<usize, LookupFn>(addr) });
        }

        const XMEM_WIDE: [u16; 9] = [
            b'x' as u16,
            b'm' as u16,
            b'e' as u16,
            b'm' as u16,
            b'.' as u16,
            b'd' as u16,
            b'l' as u16,
            b'l' as u16,
            0,
        ];

        // SAFETY: `XMEM_WIDE` is NUL-terminated UTF-16.
        let mut module = unsafe { GetModuleHandleW(XMEM_WIDE.as_ptr()) };
        if module.is_null() {
            let mut self_mod: *mut c_void = std::ptr::null_mut();
            // SAFETY: `lookup_fn` lies inside this DLL; `self_mod` is writable.
            let ok = unsafe {
                GetModuleHandleExW(
                    GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS
                        | GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT,
                    (lookup_fn as *const ()).cast(),
                    &mut self_mod,
                )
            };
            if ok != 0 && !self_mod.is_null() {
                let mut path = vec![0u16; 32768];
                // SAFETY: `path` is writable for `path.len()` UTF-16 code units.
                let len = unsafe {
                    GetModuleFileNameW(self_mod, path.as_mut_ptr(), path.len() as u32)
                } as usize;
                if let Some(slash) = (len > 0 && len < path.len())
                    .then(|| {
                        path[..len]
                            .iter()
                            .rposition(|&c| c == b'\\' as u16 || c == b'/' as u16)
                    })
                    .flatten()
                {
                    let mut sibling = Vec::with_capacity(slash + 1 + XMEM_WIDE.len());
                    sibling.extend_from_slice(&path[..=slash]);
                    sibling.extend_from_slice(&XMEM_WIDE);
                    // SAFETY: `sibling` is NUL-terminated UTF-16.
                    module = unsafe { LoadLibraryW(sibling.as_ptr()) };
                }
            }
        }
        if module.is_null() {
            // SAFETY: `XMEM_WIDE` is NUL-terminated UTF-16.
            module = unsafe { LoadLibraryW(XMEM_WIDE.as_ptr()) };
        }
        if module.is_null() {
            return None;
        }
        // SAFETY: `module` is loaded and the symbol name is NUL-terminated ASCII.
        let addr = unsafe { GetProcAddress(module, c"victory_hle_lookup_export".as_ptr()) } as usize;
        if addr == 0 {
            return None;
        }
        let _ = LOOKUP.set(addr);
        // SAFETY: `victory_hle_lookup_export` has `LookupFn`'s exact ABI.
        Some(unsafe { std::mem::transmute::<usize, LookupFn>(addr) })
    }

    pub(crate) fn resolve(dll: &CStr, symbol: &CStr) -> Option<usize> {
        let lookup = lookup_fn()?;
        // SAFETY: `dll` and `symbol` are valid NUL-terminated C strings.
        let addr = unsafe { lookup(dll.as_ptr(), symbol.as_ptr()) };
        (addr != 0).then_some(addr)
    }
}

#[cfg(not(windows))]
mod imp {
    use std::ffi::CStr;

    pub(crate) fn resolve(_dll: &CStr, _symbol: &CStr) -> Option<usize> {
        None
    }
}

pub(crate) fn cached(slot: &OnceLock<usize>, dll: &CStr, symbol: &CStr) -> Option<usize> {
    if let Some(&addr) = slot.get() {
        return Some(addr);
    }
    let addr = imp::resolve(dll, symbol)?;
    let _ = slot.set(addr);
    Some(addr)
}
