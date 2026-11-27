//! Host work that a host library records into a title's command list in
//! place of a console one (the host's FidelityFX upscaler, say): the host
//! device, a title texture's host copy in the state the library is told,
//! and the host list itself.

use windows::Win32::Graphics::Direct3D12::{
    ID3D12Device, ID3D12GraphicsCommandList, ID3D12Resource,
};

use crate::object::This;

/// The host D3D12 device, created on first use; `None` without a host GPU.
pub fn host_device() -> Option<ID3D12Device> {
    crate::host::host().map(|h| h.device.clone())
}

/// PCI vendor ID of the host adapter (`0x10DE` NVIDIA, `0x1002` AMD, etc.).
pub fn adapter_vendor_id() -> Option<u32> {
    crate::host::host().and_then(|h| h.vendor_id())
}

/// Dedicated local VRAM budget reported by DXGI for the host adapter, in bytes.
pub fn adapter_vram_budget() -> Option<u64> {
    crate::host::host()
        .and_then(|h| h.video_memory())
        .map(|[budget, ..]| budget)
}

/// Estimated system-RAM commit held by VIctory's own D3D12 emulation layers
/// (CPU shadow copies, texture staging buffers, list scratch chunks, and
/// command-list allocators), excluding the title's own allocations.
pub fn host_commit_bytes() -> u64 {
    crate::host::host_overhead_bytes()
}

/// Full `(x, y, width, height)` monitor bounds of the primary display in
/// virtual desktop coordinates, for borderless fullscreen sizing.
pub fn primary_monitor_bounds() -> Option<(i32, i32, i32, i32)> {
    use windows::Win32::Foundation::POINT;
    use windows::Win32::Graphics::Gdi::{
        GetMonitorInfoW, MONITOR_DEFAULTTOPRIMARY, MONITORINFO, MonitorFromPoint,
    };
    let mut info = MONITORINFO {
        cbSize: size_of::<MONITORINFO>() as u32,
        ..Default::default()
    };
    // SAFETY: valid POINT and sized MONITORINFO structure.
    let ok = unsafe {
        let monitor = MonitorFromPoint(POINT { x: 0, y: 0 }, MONITOR_DEFAULTTOPRIMARY);
        !monitor.is_invalid() && GetMonitorInfoW(monitor, &raw mut info).as_bool()
    };
    if !ok {
        return None;
    }
    let r = info.rcMonitor;
    Some((r.left, r.top, r.right - r.left, r.bottom - r.top))
}

/// Locate the NVIDIA NGXCore directory holding `_nvngx.dll` (`D3DKMTQueryAdapterInfo`
/// driver-store query for the active D3D12 adapter LUID, then the standard NVIDIA
/// registry key, then `System32`).
pub fn ngx_core_dir() -> Option<std::path::PathBuf> {
    if let Some(dir) = crate::host::host().and_then(|h| qai_ngx_dir(h.adapter_luid())) {
        return Some(dir);
    }
    if let Some(dir) = registry_ngx_dir() {
        return Some(dir);
    }
    if let Some(windir) = std::env::var_os("WINDIR") {
        let sys32 = std::path::PathBuf::from(windir).join("System32");
        if sys32.join("_nvngx.dll").is_file() || sys32.join("nvngx.dll").is_file() {
            return Some(sys32);
        }
    }
    None
}

fn qai_ngx_dir(luid: (u32, i32)) -> Option<std::path::PathBuf> {
    use std::ffi::c_void;
    use std::os::windows::ffi::OsStringExt;
    use windows_sys::Win32::Foundation::FreeLibrary;
    use windows_sys::Win32::System::LibraryLoader::{
        GetProcAddress, LOAD_LIBRARY_SEARCH_SYSTEM32, LoadLibraryExW,
    };

    #[repr(C)]
    #[derive(Clone, Copy)]
    struct AdapterLuid {
        low: u32,
        high: i32,
    }
    #[repr(C)]
    #[derive(Clone, Copy)]
    struct AdapterInfo {
        h_adapter: u32,
        luid: AdapterLuid,
        num_sources: u32,
        present_move_preferred: i32,
    }
    #[repr(C)]
    struct EnumAdapters2 {
        num_adapters: u32,
        adapters: *mut AdapterInfo,
    }
    #[repr(C)]
    struct QueryAdapterInfo {
        h_adapter: u32,
        kind: u32,
        private_data: *mut c_void,
        private_size: u32,
    }
    #[repr(C)]
    struct CloseAdapter {
        h_adapter: u32,
    }
    #[repr(C)]
    struct QueryRegistryHeader {
        query_type: u32,
        query_flags: u32,
        value_name: [u16; 260],
        value_type: u32,
        physical_adapter_index: u32,
        output_value_size: u32,
        status: u32,
    }
    #[repr(C)]
    struct QueryRegistryBuf {
        header: QueryRegistryHeader,
        output: [u16; 520],
    }

    type EnumFn = unsafe extern "system" fn(*mut EnumAdapters2) -> i32;
    type QueryFn = unsafe extern "system" fn(*mut QueryAdapterInfo) -> i32;
    type CloseFn = unsafe extern "system" fn(*mut CloseAdapter) -> i32;

    let gdi_name: Vec<u16> = "gdi32.dll\0".encode_utf16().collect();
    // SAFETY: loads system gdi32.dll, queries D3DKMT entry points, closes adapter handles, and frees library.
    unsafe {
        let gdi = LoadLibraryExW(
            gdi_name.as_ptr(),
            std::ptr::null_mut(),
            LOAD_LIBRARY_SEARCH_SYSTEM32,
        );
        if gdi.is_null() {
            return None;
        }
        let enum_fn = GetProcAddress(gdi, c"D3DKMTEnumAdapters2".as_ptr().cast())
            .map(|p| std::mem::transmute::<unsafe extern "system" fn() -> isize, EnumFn>(p));
        let query_fn = GetProcAddress(gdi, c"D3DKMTQueryAdapterInfo".as_ptr().cast())
            .map(|p| std::mem::transmute::<unsafe extern "system" fn() -> isize, QueryFn>(p));
        let close_fn = GetProcAddress(gdi, c"D3DKMTCloseAdapter".as_ptr().cast())
            .map(|p| std::mem::transmute::<unsafe extern "system" fn() -> isize, CloseFn>(p));
        let (Some(enum_fn), Some(query_fn), Some(close_fn)) = (enum_fn, query_fn, close_fn) else {
            let _ = FreeLibrary(gdi);
            return None;
        };

        let mut list = EnumAdapters2 {
            num_adapters: 0,
            adapters: std::ptr::null_mut(),
        };
        if enum_fn(&raw mut list) < 0 || list.num_adapters == 0 {
            let _ = FreeLibrary(gdi);
            return None;
        }
        let mut infos = vec![
            AdapterInfo {
                h_adapter: 0,
                luid: AdapterLuid { low: 0, high: 0 },
                num_sources: 0,
                present_move_preferred: 0,
            };
            list.num_adapters as usize
        ];
        list.adapters = infos.as_mut_ptr();
        let mut found = None;
        if enum_fn(&raw mut list) >= 0 {
            let matching = infos
                .iter()
                .take(list.num_adapters as usize)
                .find(|a| a.luid.low == luid.0 && a.luid.high == luid.1)
                .map(|a| a.h_adapter);
            if let Some(h_adapter) = matching {
                const KMTQAITYPE_QUERYREGISTRY: u32 = 48;
                const QUERY_REG_SZ: u32 = 1;
                let mut buf = QueryRegistryBuf {
                    header: std::mem::zeroed(),
                    output: [0u16; 520],
                };
                for key in ["Parameters\\NGXCore\\NGXPath", "NGXCore\\NGXPath"] {
                    buf.header = std::mem::zeroed();
                    buf.output.fill(0);
                    buf.header.value_type = QUERY_REG_SZ;
                    for (dst, src) in buf.header.value_name.iter_mut().zip(key.encode_utf16()) {
                        *dst = src;
                    }
                    let mut q = QueryAdapterInfo {
                        h_adapter,
                        kind: KMTQAITYPE_QUERYREGISTRY,
                        private_data: (&raw mut buf).cast(),
                        private_size: size_of::<QueryRegistryBuf>() as u32,
                    };
                    if query_fn(&raw mut q) >= 0 && buf.header.status == 0 {
                        let len = buf
                            .output
                            .iter()
                            .position(|&c| c == 0)
                            .unwrap_or(buf.output.len());
                        let path = std::path::PathBuf::from(std::ffi::OsString::from_wide(
                            &buf.output[..len],
                        ));
                        if path.is_dir() {
                            found = Some(path);
                            break;
                        }
                    }
                }
            }
            for info in infos.iter().take(list.num_adapters as usize) {
                let mut close = CloseAdapter {
                    h_adapter: info.h_adapter,
                };
                let _ = close_fn(&raw mut close);
            }
        }
        let _ = FreeLibrary(gdi);
        found
    }
}

fn registry_ngx_dir() -> Option<std::path::PathBuf> {
    use std::os::windows::ffi::OsStringExt;
    use windows_sys::Win32::System::Registry::{
        HKEY, HKEY_LOCAL_MACHINE, KEY_READ, RegCloseKey, RegOpenKeyExW, RegQueryValueExW,
    };
    const REG_SZ_KIND: u32 = 1;
    let sub: Vec<u16> = "SOFTWARE\\NVIDIA Corporation\\Global\\NGXCore\0"
        .encode_utf16()
        .collect();
    let val: Vec<u16> = "FullPath\0".encode_utf16().collect();
    let mut key: HKEY = std::ptr::null_mut();
    // SAFETY: NUL-terminated registry paths and sized output buffer.
    unsafe {
        if RegOpenKeyExW(HKEY_LOCAL_MACHINE, sub.as_ptr(), 0, KEY_READ, &raw mut key) != 0 {
            return None;
        }
        let mut kind = 0u32;
        let mut buf = vec![0u16; 520];
        let mut bytes = (buf.len() * size_of::<u16>()) as u32;
        let res = RegQueryValueExW(
            key,
            val.as_ptr(),
            std::ptr::null(),
            &raw mut kind,
            buf.as_mut_ptr().cast(),
            &raw mut bytes,
        );
        let _ = RegCloseKey(key);
        if res != 0 || kind != REG_SZ_KIND {
            return None;
        }
        let units = (bytes as usize / 2).min(buf.len());
        let len = buf[..units].iter().position(|&c| c == 0).unwrap_or(units);
        let path = std::path::PathBuf::from(std::ffi::OsString::from_wide(&buf[..len]));
        path.is_dir().then_some(path)
    }
}

/// The active host list for one synchronous external recording call.
pub struct Recording<'a>(&'a crate::host::HostList);

impl Recording<'_> {
    /// Move a title texture to the state the provider will see. The list
    /// holds the backing resource through submission.
    pub fn texture(&self, resource: This, state: u32) -> Option<(ID3D12Resource, u32)> {
        let texture = crate::resource::host_resource(resource)?.clone();
        crate::list::ensure_state(self.0, resource, state);
        let state = crate::host::legal_state(&texture, state);
        Some((texture, state))
    }

    /// Prepare aliases, mip uploads and depth twins like a compute SRV.
    pub fn prepare_input(&self, resource: This) -> Option<()> {
        let texture = crate::resource::host_resource(resource)?;
        let aliased = crate::resource::alias_of(resource)
            .is_some_and(|source| crate::texture::alias(self.0, resource, source));
        if !aliased && crate::texture::title_written(texture) {
            crate::texture::upload_from(self.0, resource, 0);
        }
        self.0.twin_use(resource, false, false);
        Some(())
    }

    /// Access flushes any pending host barriers before the provider runs.
    pub fn commands(&self) -> &ID3D12GraphicsCommandList {
        &self.0.list
    }
}

/// The producer recorded its output on the host's command list.
pub fn output_written(resource: This) {
    crate::texture::host_written(resource);
}

/// Settle submitted title work before releasing a provider's private GPU resources.
pub fn finish_submitted() {
    crate::queue::before_sync();
    crate::host::settle_for_fence();
}

/// Runs `f` with the host list behind the title list `list` (open for
/// recording); `None` without one. What `f` binds (descriptor heaps, root
/// signatures, pipeline) is replaced before the list's next draw or
/// dispatch.
pub fn record<R>(list: This, f: impl FnOnce(&Recording<'_>) -> R) -> Option<R> {
    let list = crate::list::thin::for_external(list)?;
    let mut out = None;
    crate::list::with_state(list, |host, state| {
        out = Some(f(&Recording(host)));
        state.external_compute();
    });
    out
}
