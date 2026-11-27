//! `amd_fidelityfx_loader_xbox.dll`: AMD FidelityFX's API, served by the
//! host's PC build of it on the host GPU when the user names a folder that
//! holds one (`VICTORY_FIDELITYFX_DIR`) and selects a 4K FSR profile.
//! Other profiles keep the title's own console DLLs.
//!
//! Both builds have the same API (the FidelityFX SDK's, MIT): five functions
//! over chains of descriptions, each starting with a type id and the next
//! one's address; contexts are opaque pointers. The console-specific parts
//! are translated: the backend description names the console device (type
//! 0x0100_0002; on PC type 2 with the host device), and a dispatch names the
//! title's command list and resources (their host list and host copies, in
//! the states the title gives, see `victory_gpu::external`). The rest passes
//! through unchanged: sizes, flags, jitter, motion-vector scale, camera
//! values, the resource descriptions, the context. Only the upscaler: other
//! effects (frame generation) are refused.
#![allow(non_snake_case)]
// Title code calls these through its import table.
#![allow(clippy::not_unsafe_ptr_arg_deref)]

use std::collections::BTreeMap;
use std::ffi::{CStr, c_char, c_void};
use std::path::Path;
use std::ptr::{null, null_mut};
use std::sync::{Arc, Mutex, OnceLock};

use victory_gpu::object::Object;
use windows::Win32::Graphics::Direct3D12::ID3D12Device;
use windows::core::Interface;

use crate::{HleModule, hle_export};

/// The PC build's upscaler, which has the five functions itself.
const DLL: &str = "amd_fidelityfx_upscaler_dx12.dll";

// Return codes.
const OK: u32 = 0;
const UNKNOWN_TYPE: u32 = 2;
const NO_PROVIDER: u32 = 4;
const PARAMETER: u32 = 6;

// Description types.
const CREATE_UPSCALE: u64 = 0x1_0000;
const DISPATCH_UPSCALE: u64 = 0x1_0001;
const CONSOLE_BACKEND: u64 = 0x100_0002;
const HOST_BACKEND: u64 = 2;
const GET_VERSIONS: u64 = 4;
const OVERRIDE_VERSION: u64 = 5;
const GET_PROVIDER_VERSION: u64 = 6;
const MEMORY_USAGE: u64 = 0x1_0008;
const MEMORY_USAGE_WITH_DEVICE: u64 = 0x1_0009;
const GET_UPSCALE_RATIO: u64 = 0x1_0002;
const GET_RENDER_RESOLUTION: u64 = 0x1_0003;
const GET_JITTER_PHASE_COUNT: u64 = 0x1_0004;
const GET_JITTER_OFFSET: u64 = 0x1_0005;
const DISPLAY_RESOLUTION_MOTION_VECTORS: u32 = 1 << 1;

const DLSS_PROVIDER_ID: u64 = 0x0044_4C53_5300_0001;
const DLSS_PROVIDER_NAME: &CStr = c"DLSS 3/4";

#[repr(C)]
#[derive(Clone, Copy)]
struct Header {
    kind: u64,
    next: *mut Header,
}

/// An upscaler context: flags, largest render and output sizes, a message
/// callback.
#[repr(C)]
#[derive(Clone, Copy)]
struct CreateUpscale {
    header: Header,
    flags: u32,
    max_render: [u32; 2],
    max_output: [u32; 2],
    message: usize,
}

/// The backend description: a device.
#[repr(C)]
struct Backend {
    header: Header,
    device: *mut c_void,
}

/// A resource, its description (type, format, sizes, mips, flags, usage)
/// and the state it is in.
#[repr(C)]
#[derive(Clone, Copy)]
struct Resource {
    resource: *mut c_void,
    description: [u32; 8],
    state: u32,
}

/// One upscaling: the command list, then colour, depth, motion vectors,
/// exposure, reactive mask, transparency mask and output; then jitter,
/// motion-vector scale, sizes, sharpening, frame time, pre-exposure, reset,
/// camera and flags, which pass through.
#[repr(C)]
#[derive(Clone, Copy)]
struct DispatchUpscale {
    header: Header,
    list: *mut c_void,
    resources: [Resource; 7],
    jitter: [f32; 2],
    motion_scale: [f32; 2],
    render_size: [u32; 2],
    upscale_size: [u32; 2],
    settings: [u32; 10],
}

const _: () = assert!(size_of::<CreateUpscale>() == 0x30);
const _: () = assert!(size_of::<Resource>() == 0x30);
const _: () = assert!(size_of::<DispatchUpscale>() == 0x1b0);

/// The versions a backend offers: count in and out, ids, names.
#[repr(C)]
#[derive(Clone, Copy)]
struct GetVersions {
    header: Header,
    create_kind: u64,
    device: *mut c_void,
    count: *mut u64,
    ids: *mut u64,
    names: *mut *const c_char,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct MemoryUsageWithDevice {
    header: Header,
    device: *mut c_void,
    max_render: [u32; 2],
    max_output: [u32; 2],
    flags: u32,
    usage: *mut c_void,
}

#[repr(C)]
struct ProviderVersion {
    header: Header,
    id: u64,
    name: *const c_char,
}

#[repr(C)]
struct OverrideVersion {
    header: Header,
    id: u64,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct RatioQuery {
    header: Header,
    quality: u32,
    ratio: *mut f32,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct RenderResolutionQuery {
    header: Header,
    display: [u32; 2],
    quality: u32,
    width: *mut u32,
    height: *mut u32,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct JitterPhaseCountQuery {
    header: Header,
    render_width: u32,
    output_width: u32,
    count: *mut i32,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct JitterOffsetQuery {
    header: Header,
    index: i32,
    phases: i32,
    x: *mut f32,
    y: *mut f32,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct MemoryUsageQuery {
    header: Header,
    usage: *mut [u64; 2],
}

type Create = unsafe extern "C" fn(*mut *mut c_void, *mut Header, *const c_void) -> u32;
type Destroy = unsafe extern "C" fn(*mut *mut c_void, *const c_void) -> u32;
/// Configure, query and dispatch.
type Call = unsafe extern "C" fn(*mut *mut c_void, *mut Header) -> u32;

struct Host {
    _library: Library,
    create: Create,
    destroy: Destroy,
    configure: Call,
    query: Call,
    dispatch: Call,
    device: ID3D12Device,
    provider_id: u64,
    /// The provider's version as the SDK names it (`4.1.2`, `3.1.5`).
    provider_name: String,
}

struct Library(windows_sys::Win32::Foundation::HMODULE);

impl Drop for Library {
    fn drop(&mut self) {
        // SAFETY: this owns the reference acquired by LoadLibraryExW.
        unsafe {
            windows_sys::Win32::Foundation::FreeLibrary(self.0);
        }
    }
}

// SAFETY: D3D12 devices are free-threaded; the rest is code.
unsafe impl Send for Host {}
// SAFETY: as above.
unsafe impl Sync for Host {}

enum Provider {
    FidelityFx(Host),
    Dlss,
}

static PROVIDER: OnceLock<Provider> = OnceLock::new();

enum ContextBackend {
    FidelityFx(CreateChain),
    Dlss(Mutex<super::dlss::DlssInstance>),
}

struct Context {
    // Serializes recording and destruction for this particular temporal history.
    live: Mutex<bool>,
    first_dispatch: std::sync::Once,
    backend: ContextBackend,
}

struct CreateChain {
    description: Box<CreateUpscale>,
    _backend: Box<Backend>,
    _version: Box<OverrideVersion>,
}

// SAFETY: after Create, the boxed descriptors are retained immutably.
// The device is held by Host; the remaining extension pointers belong
// to the title under the FFX context lifetime contract.
unsafe impl Send for CreateChain {}
// SAFETY: as above, no thread mutates the retained descriptors.
unsafe impl Sync for CreateChain {}

static CONTEXTS: Mutex<BTreeMap<usize, Arc<Context>>> = Mutex::new(BTreeMap::new());

fn fits(extent: [u32; 2], maximum: [u32; 2]) -> bool {
    extent[0] != 0 && extent[1] != 0 && extent[0] <= maximum[0] && extent[1] <= maximum[1]
}

fn dispatch_output(
    render: [u32; 2],
    output: [u32; 2],
    max_render: [u32; 2],
    max_output: [u32; 2],
) -> Option<[u32; 2]> {
    let output = if output == [0, 0] { max_output } else { output };
    (fits(render, max_render) && fits(output, max_output) && fits(render, output)).then_some(output)
}

fn minimum_extent(index: usize, flags: u32, render: [u32; 2], output: [u32; 2]) -> [u32; 2] {
    match index {
        6 => output,
        2 if flags & DISPLAY_RESOLUTION_MOTION_VECTORS != 0 => output,
        3 => [1, 1], // Optional exposure texture.
        _ => render,
    }
}

macro_rules! once {
    ($level:ident, $($arg:tt)*) => {{
        static ONCE: std::sync::Once = std::sync::Once::new();
        ONCE.call_once(|| tracing::$level!($($arg)*));
    }};
}

/// Select the PC FidelityFX provider before the title's import registry is initialized.
pub fn use_host_dir(dir: &Path) -> Result<(), String> {
    if PROVIDER.get().is_some() {
        return Err(String::from("Upscaler provider already initialized"));
    }
    let (host, version) = load(dir)?;
    PROVIDER
        .set(Provider::FidelityFx(host))
        .map_err(|_| String::from("Upscaler provider already initialized"))?;
    if !is_fsr_4_1_2(&version) {
        tracing::warn!(
            version,
            "FidelityFX: the SDK offers this adapter no FSR 4.1.2; FSR 3.1 is used in its place (its picture differs from 4's)"
        );
    }
    tracing::info!(folder = %dir.display(), version, "FidelityFX: selected PC upscaler");
    Ok(())
}

/// Select the NVIDIA DLSS provider before the title's import registry is initialized.
pub fn use_dlss(dir: Option<&Path>) -> Result<(), String> {
    if PROVIDER.get().is_some() {
        return Err(String::from("Upscaler provider already initialized"));
    }
    super::dlss::init(dir)?;
    PROVIDER
        .set(Provider::Dlss)
        .map_err(|_| String::from("Upscaler provider already initialized"))?;
    let resolved = super::dlss::dlss_dir()
        .map(|p| p.display().to_string())
        .unwrap_or_else(|| String::from("auto"));
    tracing::info!(folder = resolved, "NVIDIA DLSS: selected PC upscaler");
    Ok(())
}

/// Whether a host upscaler serves `amd_fidelityfx_loader_xbox.dll`.
pub fn serving() -> bool {
    PROVIDER.get().is_some()
}

fn load(dir: &Path) -> Result<(Host, String), String> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::System::LibraryLoader::{
        GetProcAddress, LOAD_LIBRARY_SEARCH_DLL_LOAD_DIR, LOAD_LIBRARY_SEARCH_SYSTEM32,
        LoadLibraryExW,
    };
    // The search flags below want a full path.
    let path = std::path::absolute(dir.join(DLL)).map_err(|e| e.to_string())?;
    if !path.is_file() {
        return Err(format!("no {DLL} there"));
    }
    let wide: Vec<u16> = path.as_os_str().encode_wide().chain([0]).collect();
    // Its own folder and System32 only, never the current folder.
    let search = LOAD_LIBRARY_SEARCH_DLL_LOAD_DIR | LOAD_LIBRARY_SEARCH_SYSTEM32;
    // SAFETY: a NUL-terminated path to the user's FidelityFX build.
    let module = unsafe { LoadLibraryExW(wide.as_ptr(), null_mut(), search) };
    if module.is_null() {
        return Err(format!(
            "{DLL} did not load ({})",
            std::io::Error::last_os_error()
        ));
    }
    let library = Library(module);
    let find = |name: &CStr| {
        // SAFETY: a loaded module and a NUL-terminated name.
        unsafe { GetProcAddress(module, name.as_ptr().cast()) }
            .map(|f| f as *const c_void)
            .ok_or_else(|| format!("{DLL} has no {}", name.to_string_lossy()))
    };
    let device = victory_gpu::external::host_device().ok_or("there is no host GPU")?;
    // SAFETY: the SDK's ffxQuery export, with its declared signature.
    let query = unsafe { std::mem::transmute::<*const c_void, Call>(find(c"ffxQuery")?) };
    let (provider_id, version) = select_version(query, &device)?;
    // SAFETY: the API's exports, with its signatures.
    let host = unsafe {
        Host {
            create: std::mem::transmute::<*const c_void, Create>(find(c"ffxCreateContext")?),
            destroy: std::mem::transmute::<*const c_void, Destroy>(find(c"ffxDestroyContext")?),
            configure: std::mem::transmute::<*const c_void, Call>(find(c"ffxConfigure")?),
            query,
            dispatch: std::mem::transmute::<*const c_void, Call>(find(c"ffxDispatch")?),
            _library: library,
            device,
            provider_id,
            provider_name: version.clone(),
        }
    };
    Ok((host, version))
}

fn is_fsr_4_1_2(name: &str) -> bool {
    name.strip_prefix("4.1.2")
        .is_some_and(|suffix| !suffix.starts_with(|c: char| c.is_ascii_digit()))
}

/// The SDK supplies the version id to put in the context creation chain.
fn select_version(query_fn: Call, device: &ID3D12Device) -> Result<(u64, String), String> {
    let mut count = 0u64;
    let mut query = GetVersions {
        header: Header {
            kind: GET_VERSIONS,
            next: null_mut(),
        },
        create_kind: CREATE_UPSCALE,
        device: device.as_raw(),
        count: &mut count,
        ids: null_mut(),
        names: null_mut(),
    };
    // SAFETY: a version query naming the host device.
    let code = unsafe { query_fn(null_mut(), &mut query.header) };
    if code != OK || count == 0 {
        return Err(format!(
            "it offers no upscaler for the host GPU (code {code})"
        ));
    }
    let mut names = vec![null::<c_char>(); count as usize];
    let mut ids = vec![0u64; names.len()];
    query.names = names.as_mut_ptr();
    query.ids = ids.as_mut_ptr();
    // SAFETY: as above, with room for `count` names.
    let code = unsafe { query_fn(null_mut(), &mut query.header) };
    if code != OK {
        return Err(format!(
            "upscaler version names could not be queried (code {code})"
        ));
    }
    let mut offered = Vec::new();
    for (id, name) in ids.into_iter().zip(names).take(count as usize) {
        if !name.is_null() {
            // SAFETY: NUL-terminated name owned by the SDK.
            let name = unsafe { CStr::from_ptr(name) }
                .to_string_lossy()
                .into_owned();
            offered.push((id, name));
        }
    }
    choose(offered)
}

/// The patch number of an FSR 3.1 provider's name (`3.1.5` → 5); `None` for
/// any other version (`3.10.0`, `2.3.4`, `4.1.2`).
fn fsr_3_1_patch(name: &str) -> Option<u32> {
    let patch = name.strip_prefix("3.1.")?;
    let digits = patch.len() - patch.trim_start_matches(|c: char| c.is_ascii_digit()).len();
    patch[..digits].parse().ok()
}

/// The provider to ask for, of those the SDK offers for the adapter: FSR
/// 4.1.2 where there is one, otherwise the newest FSR 3.1 (which runs on any
/// D3D12 card; AMD offers 4 on few). With neither, what was offered is said.
fn choose(offered: Vec<(u64, String)>) -> Result<(u64, String), String> {
    if let Some(at) = offered.iter().position(|(_, name)| is_fsr_4_1_2(name)) {
        return Ok(offered.into_iter().nth(at).unwrap());
    }
    let newest = offered
        .iter()
        .enumerate()
        .filter_map(|(at, (_, name))| fsr_3_1_patch(name).map(|patch| (patch, at)))
        .max_by_key(|&(patch, _)| patch);
    match newest {
        Some((_, at)) => Ok(offered.into_iter().nth(at).unwrap()),
        None => {
            let names: Vec<&str> = offered.iter().map(|(_, name)| name.as_str()).collect();
            Err(format!(
                "the SDK offers this adapter neither FSR 4.1.2 nor FSR 3.1 (offered: {}); run without -Fsr4K (VICTORY_FSR_4K)",
                if names.is_empty() { "nothing".into() } else { names.join(", ") }
            ))
        }
    }
}

/// Puts the host backend after the title's upscaler description, in place
/// of the console's backend that follows it (where the title puts it);
/// anywhere else the console's is left for the host DLL to ignore, ours
/// coming first.
fn link(upscale: &mut CreateUpscale, backend: &mut Backend) {
    let after = upscale.header.next;
    // SAFETY: the title's chain.
    backend.header.next = match unsafe { after.as_ref() } {
        Some(h) if h.kind == CONSOLE_BACKEND => h.next,
        _ => after,
    };
    backend.header.kind = HOST_BACKEND;
    upscale.header.next = &raw mut backend.header;
}

/// A FidelityFX resource state as the host resource state it means (as the
/// PC backend reads it); `None` for generic read and indirect arguments,
/// which a host texture cannot be in.
fn host_state(state: u32) -> Option<u32> {
    Some(match state {
        0x1 | 0x80 => 0,  // common, present
        0x2 => 0x8,       // unordered access
        0x4 => 0x40,      // compute read
        0x8 => 0x80,      // pixel read
        0xc => 0xc0,      // both
        0x10 => 0x800,    // copy source
        0x20 => 0x400,    // copy destination
        0x100 => 0x4,     // render target
        0x200 => 0x10,    // depth write
        _ => return None, // generic read, indirect arguments
    })
}

fn dlss_input_state(state: u32) -> Option<u32> {
    let base = host_state(state)?;
    Some(match base {
        0 => 0x40,
        0x80 => 0xc0,
        other => other,
    })
}

extern "C" fn ffxCreateContext(
    context: *mut *mut c_void,
    desc: *mut Header,
    memory: *const c_void,
) -> u32 {
    let Some(provider) = PROVIDER.get() else {
        return NO_PROVIDER;
    };
    if context.is_null() || desc.is_null() {
        return PARAMETER;
    }
    // SAFETY: the title passes a chain of descriptions.
    let kind = unsafe { (*desc).kind };
    if kind != CREATE_UPSCALE {
        once!(
            warn,
            kind = format_args!("{kind:#x}"),
            "FidelityFX: only the upscaler runs on the host; another effect was refused"
        );
        return NO_PROVIDER;
    }
    // SAFETY: an upscaler description, by its type.
    let mut upscale = Box::new(unsafe { desc.cast::<CreateUpscale>().read() });
    match provider {
        Provider::Dlss => {
            let instance = match super::dlss::DlssInstance::new(
                upscale.flags,
                upscale.max_render,
                upscale.max_output,
            ) {
                Ok(inst) => inst,
                Err(err) => {
                    tracing::error!(
                        %err,
                        render = ?upscale.max_render,
                        output = ?upscale.max_output,
                        flags = format_args!("{:#x}", upscale.flags),
                        "DLSS: failed to allocate upscaler context"
                    );
                    return NO_PROVIDER;
                }
            };
            let handle = Box::into_raw(Box::new(0usize)).cast::<c_void>();
            // SAFETY: caller passes writable context slot.
            unsafe { *context = handle };
            tracing::info!(
                provider = "DLSS 3/4",
                render = ?upscale.max_render,
                output = ?upscale.max_output,
                flags = format_args!("{:#x}", upscale.flags),
                "FidelityFX: DLSS upscaler context on the host"
            );
            CONTEXTS.lock().unwrap_or_else(|e| e.into_inner()).insert(
                handle as usize,
                Arc::new(Context {
                    live: Mutex::new(true),
                    first_dispatch: std::sync::Once::new(),
                    backend: ContextBackend::Dlss(Mutex::new(instance)),
                }),
            );
            OK
        }
        Provider::FidelityFx(host) => {
            let mut backend = Box::new(Backend {
                header: Header {
                    kind: HOST_BACKEND,
                    next: null_mut(),
                },
                device: host.device.as_raw(),
            });
            link(&mut upscale, &mut backend);
            let mut requested = Box::new(OverrideVersion {
                header: Header {
                    kind: OVERRIDE_VERSION,
                    next: backend.header.next,
                },
                id: host.provider_id,
            });
            backend.header.next = &raw mut requested.header;
            // SAFETY: a PC chain; the title's later descriptions live for the call.
            let code = unsafe { (host.create)(context, &mut upscale.header, memory) };
            if code != OK {
                tracing::error!(code, render = ?upscale.max_render, output = ?upscale.max_output, flags = format_args!("{:#x}", upscale.flags), "FidelityFX: the host refused the title's upscaler");
                return code;
            }
            let mut version = ProviderVersion {
                header: Header {
                    kind: GET_PROVIDER_VERSION,
                    next: null_mut(),
                },
                id: 0,
                name: null(),
            };
            // SAFETY: a provider query on the context just made.
            let query_code = unsafe { (host.query)(context, &mut version.header) };
            // SAFETY: a NUL-terminated name the DLL keeps, if any.
            let name = (!version.name.is_null())
                .then(|| unsafe { CStr::from_ptr(version.name) }.to_string_lossy());
            if query_code != OK
                || version.id != host.provider_id
                || name.as_deref() != Some(host.provider_name.as_str())
            {
                tracing::warn!(
                    query_code,
                    provider = name.as_deref().unwrap_or("?"),
                    requested = host.provider_name,
                    "FidelityFX created a different provider than the one requested"
                );
                // SAFETY: the PC provider just created the context with this allocator.
                unsafe { (host.destroy)(context, memory) };
                return NO_PROVIDER;
            }
            // SAFETY: a successful SDK creation wrote an opaque context into this slot.
            tracing::info!(
                provider = name.as_deref().unwrap_or("?"),
                id = format_args!("{:#x}", version.id),
                render = ?upscale.max_render,
                output = ?upscale.max_output,
                flags = format_args!("{:#x}", upscale.flags),
                "FidelityFX: upscaler context on the host"
            );
            CONTEXTS.lock().unwrap_or_else(|e| e.into_inner()).insert(
                unsafe { *context } as usize,
                Arc::new(Context {
                    live: Mutex::new(true),
                    first_dispatch: std::sync::Once::new(),
                    backend: ContextBackend::FidelityFx(CreateChain {
                        description: upscale,
                        _backend: backend,
                        _version: requested,
                    }),
                }),
            );
            OK
        }
    }
}

extern "C" fn ffxDestroyContext(context: *mut *mut c_void, memory: *const c_void) -> u32 {
    let Some(provider) = PROVIDER.get() else {
        return NO_PROVIDER;
    };
    if context.is_null() {
        return PARAMETER;
    }
    // SAFETY: the title passes its context. Capture its value before destroy clears it.
    let handle = unsafe { *context } as usize;
    let ctx = CONTEXTS
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get(&handle)
        .cloned();
    let Some(ctx) = ctx else { return PARAMETER };
    let mut live = ctx.live.lock().unwrap_or_else(|e| e.into_inner());
    if !*live {
        return PARAMETER;
    }
    match provider {
        Provider::Dlss => {
            *live = false;
            drop(live);
            let mut contexts = CONTEXTS.lock().unwrap_or_else(|e| e.into_inner());
            if contexts
                .get(&handle)
                .is_some_and(|current| Arc::ptr_eq(current, &ctx))
            {
                contexts.remove(&handle);
            }
            drop(contexts);
            drop(ctx);
            if handle != 0 {
                // SAFETY: synthetic box allocated in `ffxCreateContext` for DLSS handle uniqueness.
                unsafe {
                    drop(Box::from_raw(handle as *mut usize));
                    *context = null_mut();
                }
            }
            OK
        }
        Provider::FidelityFx(host) => {
            // The guest releases a context after submitting its frames. The host
            // may still be executing their lists, even after guest Execute returns.
            victory_gpu::external::finish_submitted();
            // SAFETY: live SDK context and allocator.
            let code = unsafe { (host.destroy)(context, memory) };
            if code == OK {
                *live = false;
                let mut contexts = CONTEXTS.lock().unwrap_or_else(|e| e.into_inner());
                // A concurrent Create may already have reused the released SDK address.
                if contexts
                    .get(&handle)
                    .is_some_and(|current| Arc::ptr_eq(current, &ctx))
                {
                    contexts.remove(&handle);
                }
            }
            code
        }
    }
}

/// Settings (key-value pairs, debug messages) name nothing console-specific.
extern "C" fn ffxConfigure(context: *mut *mut c_void, desc: *mut Header) -> u32 {
    match PROVIDER.get() {
        Some(Provider::FidelityFx(h)) => {
            // SAFETY: the title passes its context and descriptions.
            unsafe { (h.configure)(context, desc) }
        }
        Some(Provider::Dlss) => OK,
        None => NO_PROVIDER,
    }
}

extern "C" fn ffxQuery(context: *mut *mut c_void, desc: *mut Header) -> u32 {
    let Some(provider) = PROVIDER.get() else {
        return NO_PROVIDER;
    };
    if desc.is_null() {
        return PARAMETER;
    }
    // SAFETY: the title passes a description.
    let kind = unsafe { (*desc).kind };
    match provider {
        Provider::Dlss => dlss_query(kind, desc),
        Provider::FidelityFx(host) => {
            // Substitute the native device; the SDK returns values through the
            // title's original output pointers in both request structures.
            if kind == GET_VERSIONS {
                // SAFETY: caller passes a valid GetVersions description.
                let mut query = unsafe { *desc.cast::<GetVersions>() };
                query.device = host.device.as_raw();
                // SAFETY: forwarded with valid device.
                return unsafe { (host.query)(context, &mut query.header) };
            }
            if kind == MEMORY_USAGE_WITH_DEVICE {
                // SAFETY: caller passes a valid MemoryUsageWithDevice description.
                let mut query = unsafe { *desc.cast::<MemoryUsageWithDevice>() };
                query.device = host.device.as_raw();
                // SAFETY: forwarded with valid device.
                return unsafe { (host.query)(context, &mut query.header) };
            }
            // SAFETY: as above.
            let code = unsafe { (host.query)(context, desc) };
            if kind == GET_JITTER_PHASE_COUNT && code == OK {
                // Prevent divide-by-zero in the title at 0.5x / ultra-perf ratios
                // when an SDK returns 0 jitter phases.
                // SAFETY: verified query type `GET_JITTER_PHASE_COUNT`.
                let q = unsafe { *desc.cast::<JitterPhaseCountQuery>() };
                if !q.count.is_null() && unsafe { *q.count } <= 0 {
                    // SAFETY: checked non-null output pointer.
                    unsafe {
                        *q.count = super::dlss::jitter_phase_count(q.render_width, q.output_width);
                    }
                }
            }
            if kind == GET_RENDER_RESOLUTION && code != OK {
                tracing::warn!(
                    code,
                    "FidelityFX provider refused the render-resolution query"
                );
            }
            code
        }
    }
}

fn dlss_query(kind: u64, desc: *mut Header) -> u32 {
    // SAFETY: `desc` was checked non-null and its `kind` determines its exact `#[repr(C)]` layout.
    unsafe {
        match kind {
            GET_VERSIONS => {
                let q = &*desc.cast::<GetVersions>();
                if q.count.is_null() {
                    return PARAMETER;
                }
                *q.count = 1;
                if !q.ids.is_null() {
                    *q.ids = DLSS_PROVIDER_ID;
                }
                if !q.names.is_null() {
                    *q.names = DLSS_PROVIDER_NAME.as_ptr();
                }
                OK
            }
            GET_PROVIDER_VERSION => {
                let q = &mut *desc.cast::<ProviderVersion>();
                q.id = DLSS_PROVIDER_ID;
                q.name = DLSS_PROVIDER_NAME.as_ptr();
                OK
            }
            GET_UPSCALE_RATIO => {
                let q = &*desc.cast::<RatioQuery>();
                let Some(ratio) = super::dlss::ffx_quality_ratio(q.quality) else {
                    return PARAMETER;
                };
                if q.ratio.is_null() {
                    return PARAMETER;
                }
                *q.ratio = ratio;
                OK
            }
            GET_RENDER_RESOLUTION => {
                let q = &*desc.cast::<RenderResolutionQuery>();
                let Some([w, h]) = super::dlss::query_optimal_resolution(q.display, q.quality) else {
                    return PARAMETER;
                };
                if q.width.is_null() || q.height.is_null() {
                    return PARAMETER;
                }
                *q.width = w;
                *q.height = h;
                OK
            }
            GET_JITTER_PHASE_COUNT => {
                let q = &*desc.cast::<JitterPhaseCountQuery>();
                if q.count.is_null() {
                    return PARAMETER;
                }
                *q.count = super::dlss::jitter_phase_count(q.render_width, q.output_width);
                OK
            }
            GET_JITTER_OFFSET => {
                let q = &*desc.cast::<JitterOffsetQuery>();
                if q.x.is_null() || q.y.is_null() {
                    return PARAMETER;
                }
                let [jx, jy] = super::dlss::jitter_offset(q.index, q.phases);
                *q.x = jx;
                *q.y = jy;
                OK
            }
            MEMORY_USAGE => {
                let q = &*desc.cast::<MemoryUsageQuery>();
                if q.usage.is_null() {
                    return PARAMETER;
                }
                *q.usage = super::dlss::query_memory_usage();
                OK
            }
            MEMORY_USAGE_WITH_DEVICE => {
                let q = &*desc.cast::<MemoryUsageWithDevice>();
                if q.usage.is_null() {
                    return PARAMETER;
                }
                *q.usage.cast::<[u64; 2]>() = super::dlss::query_memory_usage();
                OK
            }
            _ => UNKNOWN_TYPE,
        }
    }
}

extern "C" fn ffxDispatch(context: *mut *mut c_void, desc: *mut Header) -> u32 {
    let Some(provider) = PROVIDER.get() else {
        return NO_PROVIDER;
    };
    if desc.is_null() {
        return PARAMETER;
    }
    // SAFETY: the title passes a description.
    let kind = unsafe { (*desc).kind };
    if kind != DISPATCH_UPSCALE {
        once!(
            warn,
            kind = format_args!("{kind:#x}"),
            "FidelityFX: only upscaling runs on the host; a dispatch was refused"
        );
        return UNKNOWN_TYPE;
    }
    // SAFETY: an upscaling, by its type.
    let mut dispatch = unsafe { desc.cast::<DispatchUpscale>().read() };
    if context.is_null() {
        once!(warn, "FidelityFX dispatch arrived without a context");
        return PARAMETER;
    }
    if let Some(index) = [0, 1, 2, 6]
        .into_iter()
        .find(|&i| dispatch.resources[i].resource.is_null())
    {
        once!(
            warn,
            index,
            "FidelityFX dispatch is missing a required texture"
        );
        return PARAMETER;
    }
    // SAFETY: the title passes the same opaque context slot as for creation.
    let handle = unsafe { *context } as usize;
    let ctx = CONTEXTS
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get(&handle)
        .cloned();
    let Some(ctx) = ctx else {
        once!(
            warn,
            "FidelityFX dispatch referenced an unregistered context"
        );
        return PARAMETER;
    };
    let live = ctx.live.lock().unwrap_or_else(|e| e.into_inner());
    if !*live {
        return PARAMETER;
    }
    let (max_render, max_output, create_flags) = match &ctx.backend {
        ContextBackend::FidelityFx(chain) => (
            chain.description.max_render,
            chain.description.max_output,
            chain.description.flags,
        ),
        ContextBackend::Dlss(inst) => inst.lock().unwrap_or_else(|e| e.into_inner()).bounds(),
    };
    let Some(output_size) = dispatch_output(
        dispatch.render_size,
        dispatch.upscale_size,
        max_render,
        max_output,
    ) else {
        once!(warn, render = ?dispatch.render_size, output = ?dispatch.upscale_size,
            max_render = ?max_render, max_output = ?max_output,
            "FidelityFX rejected dispatch dimensions outside the context's allocations");
        return PARAMETER;
    };
    let list = dispatch.list.cast::<Object>().cast_const();
    victory_gpu::external::record(list, |list| match (provider, &ctx.backend) {
        (Provider::FidelityFx(host), ContextBackend::FidelityFx(_)) => {
            record_dispatch(list, host, context, &ctx, create_flags, &mut dispatch, output_size)
        }
        (Provider::Dlss, ContextBackend::Dlss(inst)) => {
            record_dlss_dispatch(list, inst, &ctx, create_flags, &mut dispatch, output_size)
        }
        _ => PARAMETER,
    })
    .unwrap_or_else(|| {
        once!(
            warn,
            "FidelityFX could not record to the title command list"
        );
        PARAMETER
    })
}

fn record_dlss_dispatch(
    list: &victory_gpu::external::Recording<'_>,
    inst: &Mutex<super::dlss::DlssInstance>,
    ctx: &Context,
    create_flags: u32,
    dispatch: &mut DispatchUpscale,
    output_size: [u32; 2],
) -> u32 {
    let output = dispatch.resources[6].resource.cast::<Object>();
    let mut textures = Vec::new();
    let mut backing_extents = [None; 7];
    let mut raw_ptrs = [null_mut(); 7];

    for (index, r) in dispatch.resources.iter().enumerate() {
        if r.resource.is_null() {
            continue;
        }
        if index < 6 && list.prepare_input(r.resource.cast::<Object>()).is_none() {
            once!(
                warn,
                index,
                "DLSS input could not be prepared on the title command list"
            );
            return PARAMETER;
        }
        let wanted = if index == 6 {
            Some(0x8)
        } else {
            dlss_input_state(r.state)
        };
        let texture = wanted.and_then(|w| {
            list.texture(r.resource.cast::<Object>(), w)
                .filter(|&(_, state)| state == w)
        });
        let Some((texture, _)) = texture else {
            once!(
                warn,
                index,
                state = r.state,
                "DLSS: a resource has no host texture in its state; the upscaling did not run"
            );
            return PARAMETER;
        };
        // SAFETY: live host D3D12 resource.
        let desc = unsafe { texture.GetDesc() };
        backing_extents[index] = Some((desc.Width, desc.Height));
        let needed = minimum_extent(index, create_flags, dispatch.render_size, output_size);
        if desc.Width < u64::from(needed[0]) || desc.Height < needed[1] {
            once!(
                warn,
                index,
                ?needed,
                width = desc.Width,
                height = desc.Height,
                "DLSS resource cannot cover the active dispatch rectangle"
            );
            return PARAMETER;
        }
        raw_ptrs[index] = texture.as_raw();
        textures.push(texture);
    }

    let mut guard = inst.lock().unwrap_or_else(|e| e.into_inner());
    match guard.dispatch(&super::dlss::DlssEval {
        cmd_list: list.commands().as_raw(),
        resources: raw_ptrs,
        jitter: dispatch.jitter,
        motion_scale: dispatch.motion_scale,
        render_size: dispatch.render_size,
        output_size,
        settings: dispatch.settings,
    }) {
        Ok(()) => {
            victory_gpu::external::output_written(output);
            ctx.first_dispatch.call_once(|| {
                tracing::info!(
                    render = ?dispatch.render_size,
                    output = ?output_size,
                    ?backing_extents,
                    jitter = ?dispatch.jitter,
                    motion_vector_scale = ?dispatch.motion_scale,
                    "NVIDIA DLSS recorded the title's temporal dispatch"
                );
            });
            OK
        }
        Err(err) => {
            once!(warn, %err, "NVIDIA DLSS dispatch failed");
            PARAMETER
        }
    }
}

fn record_dispatch(
    list: &victory_gpu::external::Recording<'_>,
    host: &Host,
    context: *mut *mut c_void,
    ctx: &Context,
    create_flags: u32,
    dispatch: &mut DispatchUpscale,
    output_size: [u32; 2],
) -> u32 {
    let output = dispatch.resources[6].resource.cast::<Object>();
    // Alive for the call (the list holds them too).
    let mut textures = Vec::new();
    let mut backing_extents = [None; 7];
    for (index, r) in dispatch.resources.iter_mut().enumerate() {
        if r.resource.is_null() {
            continue;
        }
        if index < 6 && list.prepare_input(r.resource.cast::<Object>()).is_none() {
            once!(
                warn,
                index,
                "FidelityFX input could not be prepared on the title command list"
            );
            return PARAMETER;
        }
        let wanted = host_state(r.state);
        let texture = wanted.and_then(|w| {
            list.texture(r.resource.cast::<Object>(), w)
                .filter(|&(_, state)| state == w)
        });
        let Some((texture, _)) = texture else {
            once!(
                warn,
                index,
                state = r.state,
                "FidelityFX: a resource has no host texture in its state; the upscaling did not run"
            );
            return PARAMETER;
        };
        // SAFETY: live host D3D12 resource.
        let desc = unsafe { texture.GetDesc() };
        backing_extents[index] = Some((desc.Width, desc.Height));
        let needed = minimum_extent(
            index,
            create_flags,
            dispatch.render_size,
            output_size,
        );
        if desc.Width < u64::from(needed[0]) || desc.Height < needed[1] {
            once!(
                warn,
                index,
                ?needed,
                width = desc.Width,
                height = desc.Height,
                "FidelityFX resource cannot cover the active dispatch rectangle"
            );
            return PARAMETER;
        }
        r.resource = texture.as_raw();
        textures.push(texture);
    }
    once!(info, formats = ?textures.iter().map(|t| {
        // SAFETY: a live texture.
        unsafe { t.GetDesc() }.Format.0
    }).collect::<Vec<_>>(), "FidelityFX: first upscaling, host formats of its textures");
    dispatch.list = list.commands().as_raw();
    // SAFETY: the PC dispatch on host objects. The context is live and locked.
    let code = unsafe { (host.dispatch)(context, &raw mut dispatch.header) };
    if code == OK {
        victory_gpu::external::output_written(output);
        ctx.first_dispatch.call_once(|| {
            tracing::info!(render = ?dispatch.render_size, output = ?output_size, ?backing_extents,
                jitter = ?dispatch.jitter, motion_vector_scale = ?dispatch.motion_scale,
                "FidelityFX 4 recorded the title's temporal dispatch");
        });
    } else {
        once!(
            warn,
            code,
            "FidelityFX 4 provider rejected the title dispatch"
        );
    }
    code
}

pub(crate) fn module() -> HleModule {
    HleModule {
        name: "fidelityfx",
        dlls: &["amd_fidelityfx_loader_xbox.dll"],
        exports: vec![
            hle_export!(ffxConfigure),
            hle_export!(ffxCreateContext),
            hle_export!(ffxDestroyContext),
            hle_export!(ffxDispatch),
            hle_export!(ffxQuery),
        ],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_logical_rectangles_against_the_current_context() {
        for display in [[1920, 1080], [2560, 1440], [3840, 2160]] {
            for divisor in [1.0_f32, 1.5, 1.7, 2.0, 3.0] {
                let render = display.map(|v| (v as f32 / divisor) as u32);
                assert_eq!(
                    dispatch_output(render, display, render, display),
                    Some(display)
                );
                assert_eq!(
                    dispatch_output(render, [0, 0], render, display),
                    Some(display)
                );
                assert_eq!(minimum_extent(2, 0x29, render, display), render);
                assert_eq!(minimum_extent(2, 0x2b, render, display), display);
                assert_eq!(minimum_extent(6, 0x29, render, display), display);
                assert_eq!(minimum_extent(3, 0x29, render, display), [1, 1]);
            }
        }
        assert_eq!(
            dispatch_output([0, 720], [0, 0], [1280, 720], [2560, 1440]),
            None
        );
        assert_eq!(
            dispatch_output([1281, 720], [0, 0], [1280, 720], [2560, 1440]),
            None
        );
        assert_eq!(
            dispatch_output([1280, 720], [0, 1440], [1280, 720], [2560, 1440]),
            None
        );
        assert_eq!(
            dispatch_output([1280, 720], [3000, 1440], [1280, 720], [2560, 1440]),
            None
        );
        assert_eq!(
            dispatch_output([1280, 720], [960, 540], [1280, 720], [2560, 1440]),
            None
        );
    }

    fn header(kind: u64, next: *mut Header) -> Header {
        Header { kind, next }
    }

    /// The title's chain: upscaler, console backend, version.
    #[test]
    fn the_console_backend_becomes_the_hosts() {
        let mut version = header(0x1_000b, null_mut());
        let mut console = Backend {
            header: header(CONSOLE_BACKEND, &raw mut version),
            device: 0x1234 as *mut c_void,
        };
        let mut upscale = CreateUpscale {
            header: header(CREATE_UPSCALE, &raw mut console.header),
            flags: 0x29,
            max_render: [1280, 720],
            max_output: [1920, 1080],
            message: 0,
        };
        let mut backend = Backend {
            header: header(0, null_mut()),
            device: 0x5678 as *mut c_void,
        };
        link(&mut upscale, &mut backend);
        assert_eq!(upscale.header.next, &raw mut backend.header);
        assert_eq!(backend.header.kind, HOST_BACKEND);
        assert_eq!(backend.header.next, &raw mut version);
        assert_eq!(backend.device, 0x5678 as *mut c_void);
        // Without a console backend right after it, ours goes in front.
        let mut upscale = CreateUpscale {
            header: header(CREATE_UPSCALE, &raw mut version),
            ..upscale
        };
        link(&mut upscale, &mut backend);
        assert_eq!(backend.header.next, &raw mut version);
    }

    /// Without the PC build the title's own DLL serves its imports.
    #[test]
    fn without_the_dll_the_titles_own_runs() {
        let dir = tempfile::tempdir().unwrap();
        assert!(load(dir.path()).err().unwrap().contains(DLL));
        assert!(!serving());
        let symbol = victory_pe::ImportSymbol::Name {
            hint: 0,
            name: "ffxCreateContext".into(),
        };
        let resolver = crate::HleResolver::new(crate::HostPolicy::none());
        assert_eq!(
            resolver.classify("amd_fidelityfx_loader_xbox.dll", &symbol),
            None
        );
    }

    fn offered(names: &[&str]) -> Vec<(u64, String)> {
        names.iter().zip(1..).map(|(name, id)| (id, name.to_string())).collect()
    }

    /// FSR 4.1.2 where the SDK offers it; otherwise the newest 3.1, which
    /// is what a card without FSR 4 is offered; with neither, said so.
    #[test]
    fn the_provider_is_4_1_2_or_else_the_newest_3_1() {
        assert_eq!(choose(offered(&["3.1.5", "4.1.2", "2.3.4"])), Ok((2, "4.1.2".into())));
        assert_eq!(choose(offered(&["4.1.2 (ML)", "3.1.5"])), Ok((1, "4.1.2 (ML)".into())));
        assert_eq!(choose(offered(&["2.3.4", "3.1.4", "3.1.5"])), Ok((3, "3.1.5".into())));
        assert_eq!(choose(offered(&["3.1.10", "3.1.9"])), Ok((1, "3.1.10".into())));
        // Other versions are not taken for either.
        assert_eq!(choose(offered(&["4.1.20", "3.1.5", "3.10.0"])), Ok((2, "3.1.5".into())));
        let none = choose(offered(&["2.3.4", "4.0.3"])).unwrap_err();
        assert!(none.contains("2.3.4, 4.0.3") && none.contains("-Fsr4K"), "{none}");
        assert!(choose(Vec::new()).unwrap_err().contains("nothing"));
        assert_eq!(fsr_3_1_patch("3.1.5"), Some(5));
        assert_eq!(fsr_3_1_patch("3.1.5-beta"), Some(5));
        assert_eq!(fsr_3_1_patch("3.1."), None);
        assert_eq!(fsr_3_1_patch("3.10.0"), None);
        assert_eq!(fsr_3_1_patch("4.1.2"), None);
    }

    #[test]
    fn resource_states_read_as_the_pc_backend_reads_them() {
        assert_eq!(host_state(0xc), Some(0xc0));
        assert_eq!(host_state(0x2), Some(0x8));
        assert_eq!(host_state(0x200), Some(0x10));
        assert_eq!(host_state(0x14), None);
        assert_eq!(dlss_input_state(0x1), Some(0x40));
        assert_eq!(dlss_input_state(0x8), Some(0xc0));
        assert_eq!(dlss_input_state(0x4), Some(0x40));
    }

    #[test]
    fn dlss_queries_cover_all_fidelityfx_query_structs() {
        let mut ratio = 0.0f32;
        let mut rq = RatioQuery {
            header: Header { kind: GET_UPSCALE_RATIO, next: null_mut() },
            quality: 1,
            ratio: &raw mut ratio,
        };
        assert_eq!(dlss_query(GET_UPSCALE_RATIO, &raw mut rq.header), OK);
        assert_eq!(ratio, 1.5);

        let (mut rw, mut rh) = (0u32, 0u32);
        let mut resq = RenderResolutionQuery {
            header: Header { kind: GET_RENDER_RESOLUTION, next: null_mut() },
            display: [1920, 1080],
            quality: 1,
            width: &raw mut rw,
            height: &raw mut rh,
        };
        assert_eq!(dlss_query(GET_RENDER_RESOLUTION, &raw mut resq.header), OK);
        assert_eq!([rw, rh], [1280, 720]);

        let mut phases = 0i32;
        let mut pc = JitterPhaseCountQuery {
            header: Header { kind: GET_JITTER_PHASE_COUNT, next: null_mut() },
            render_width: 960,
            output_width: 1920,
            count: &raw mut phases,
        };
        assert_eq!(dlss_query(GET_JITTER_PHASE_COUNT, &raw mut pc.header), OK);
        assert!(phases >= 8);

        let (mut jx, mut jy) = (f32::NAN, f32::NAN);
        let mut jo = JitterOffsetQuery {
            header: Header { kind: GET_JITTER_OFFSET, next: null_mut() },
            index: 0,
            phases,
            x: &raw mut jx,
            y: &raw mut jy,
        };
        assert_eq!(dlss_query(GET_JITTER_OFFSET, &raw mut jo.header), OK);
        assert!(jx.abs() <= 0.5 && jy.abs() <= 0.5);

        let mut usage = [0u64; 2];
        let mut mem = MemoryUsageQuery {
            header: Header { kind: MEMORY_USAGE, next: null_mut() },
            usage: &raw mut usage,
        };
        assert_eq!(dlss_query(MEMORY_USAGE, &raw mut mem.header), OK);
        assert!(usage[0] > 0);
    }
}
