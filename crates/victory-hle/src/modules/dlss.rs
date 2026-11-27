//! NVIDIA DLSS Super Resolution host backend via NVIDIA NGX (`_nvngx.dll` +
//! `nvngx_dlss.dll`), exposed through the `amd_fidelityfx_loader_xbox.dll`
//! import hooks when the user selects `VICTORY_DLSS`.
//!
//! ABI verified directly against NVIDIA's `nvsdk_ngx_d.lib` (`nvsdk_ngx_defs.h`,
//! `nvsdk_ngx_params.h`, `nvsdk_ngx_loader.h`, `nvsdk_ngx_helpers_d3d.h`):
//! - `_nvngx.dll` exports flat C entry points (`NVSDK_NGX_D3D12_*`).
//! - `NVSDK_NGX_Parameter` uses MSVC x64 C++ virtual table layout where
//!   overloaded methods appear in reverse declaration order per overload group.
#![allow(non_snake_case)]
#![allow(clippy::not_unsafe_ptr_arg_deref)]

use std::ffi::{CStr, c_char, c_void};
use std::os::windows::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::ptr::{null, null_mut};
use std::sync::{Mutex, OnceLock};

use windows::Win32::Graphics::Direct3D12::ID3D12Device;
use windows::core::Interface;
use windows_sys::Win32::Foundation::{FreeLibrary, HMODULE};
use windows_sys::Win32::System::LibraryLoader::{
    GetProcAddress, LOAD_LIBRARY_SEARCH_DLL_LOAD_DIR, LOAD_LIBRARY_SEARCH_SYSTEM32,
    LOAD_WITH_ALTERED_SEARCH_PATH, LoadLibraryExW,
};

const NVSDK_NGX_VERSION_API: u32 = 0x0000_0015;
const NVSDK_NGX_FEATURE_SUPER_SAMPLING: i32 = 1;
const NVSDK_NGX_ENGINE_TYPE_CUSTOM: i32 = 0;

// NGX DLSS PerfQuality_Value enum.
const PERF_MAX_PERF: i32 = 0;
const PERF_BALANCED: i32 = 1;
const PERF_MAX_QUALITY: i32 = 2;
const PERF_ULTRA_PERF: i32 = 3;
const PERF_DLAA: i32 = 5;

// NGX DLSS feature create flags.
const NGX_FLAG_IS_HDR: i32 = 1 << 0;
const NGX_FLAG_MV_LOW_RES: i32 = 1 << 1;
const NGX_FLAG_MV_JITTERED: i32 = 1 << 2;
const NGX_FLAG_DEPTH_INVERTED: i32 = 1 << 3;
const NGX_FLAG_AUTO_EXPOSURE: i32 = 1 << 6;

// Transformer & CNN render hint presets (`NVSDK_NGX_DLSS_Hint_Render_Preset`).
const PRESET_DEFAULT: u32 = 0;
const PRESET_E: u32 = 5;
const PRESET_J: u32 = 10;
const PRESET_K: u32 = 11;
const PRESET_L: u32 = 12;
const PRESET_M: u32 = 13;
const PRESET_UNSPECIFIED_BIT: u32 = 0x8000_0000;

fn ngx_failed(code: u32) -> bool {
    (code & 0xFFF0_0000) == 0xBAD0_0000
}

#[repr(C)]
struct NvSdkNgxPathListInfo {
    path: *const *const u16,
    length: u32,
}

#[repr(C)]
struct NvSdkNgxLoggingInfo {
    logging_callback: *const c_void,
    minimum_logging_level: i32,
    disable_other_logging_sinks: bool,
}

#[repr(C)]
struct NvSdkNgxFeatureCommonInfo {
    path_list_info: NvSdkNgxPathListInfo,
    internal_data: *mut c_void,
    logging_info: NvSdkNgxLoggingInfo,
}

#[repr(C)]
pub(crate) struct NvSdkNgxHandle {
    _opaque: [u8; 0],
}

/// MSVC x64 C++ vtable of `NVSDK_NGX_Parameter`, verified against
/// `nvsdk_ngx_parameters_lib.obj` (`0x00..=0x80`).
#[repr(C)]
#[allow(dead_code)]
struct NvSdkNgxParameterVtbl {
    set_void_ptr: unsafe extern "system" fn(*mut NvSdkNgxParameter, *const c_char, *mut c_void),
    set_d3d12_res: unsafe extern "system" fn(*mut NvSdkNgxParameter, *const c_char, *mut c_void),
    set_d3d11_res: unsafe extern "system" fn(*mut NvSdkNgxParameter, *const c_char, *mut c_void),
    set_i: unsafe extern "system" fn(*mut NvSdkNgxParameter, *const c_char, i32),
    set_ui: unsafe extern "system" fn(*mut NvSdkNgxParameter, *const c_char, u32),
    set_d: unsafe extern "system" fn(*mut NvSdkNgxParameter, *const c_char, f64),
    set_f: unsafe extern "system" fn(*mut NvSdkNgxParameter, *const c_char, f32),
    set_ull: unsafe extern "system" fn(*mut NvSdkNgxParameter, *const c_char, u64),
    get_void_ptr:
        unsafe extern "system" fn(*const NvSdkNgxParameter, *const c_char, *mut *mut c_void) -> u32,
    get_d3d12_res:
        unsafe extern "system" fn(*const NvSdkNgxParameter, *const c_char, *mut *mut c_void) -> u32,
    get_d3d11_res:
        unsafe extern "system" fn(*const NvSdkNgxParameter, *const c_char, *mut *mut c_void) -> u32,
    get_i: unsafe extern "system" fn(*const NvSdkNgxParameter, *const c_char, *mut i32) -> u32,
    get_ui: unsafe extern "system" fn(*const NvSdkNgxParameter, *const c_char, *mut u32) -> u32,
    get_d: unsafe extern "system" fn(*const NvSdkNgxParameter, *const c_char, *mut f64) -> u32,
    get_f: unsafe extern "system" fn(*const NvSdkNgxParameter, *const c_char, *mut f32) -> u32,
    get_ull: unsafe extern "system" fn(*const NvSdkNgxParameter, *const c_char, *mut u64) -> u32,
    reset: unsafe extern "system" fn(*mut NvSdkNgxParameter),
}

#[repr(C)]
pub(crate) struct NvSdkNgxParameter {
    vtbl: *const NvSdkNgxParameterVtbl,
}

impl NvSdkNgxParameter {
    unsafe fn set_i(&mut self, name: &CStr, value: i32) {
        // SAFETY: valid vtable slot `0x18` on a live NGX parameter object.
        unsafe { ((*self.vtbl).set_i)(self, name.as_ptr(), value) };
    }

    unsafe fn set_ui(&mut self, name: &CStr, value: u32) {
        // SAFETY: valid vtable slot `0x20` on a live NGX parameter object.
        unsafe { ((*self.vtbl).set_ui)(self, name.as_ptr(), value) };
    }

    unsafe fn set_f(&mut self, name: &CStr, value: f32) {
        // SAFETY: valid vtable slot `0x30` on a live NGX parameter object.
        unsafe { ((*self.vtbl).set_f)(self, name.as_ptr(), value) };
    }

    unsafe fn set_res(&mut self, name: &CStr, resource: *mut c_void) {
        // SAFETY: valid vtable slot `0x08` on a live NGX parameter object.
        unsafe { ((*self.vtbl).set_d3d12_res)(self, name.as_ptr(), resource) };
    }

    unsafe fn get_i(&self, name: &CStr) -> Option<i32> {
        let mut out = 0i32;
        // SAFETY: valid vtable slot `0x58` on a live NGX parameter object.
        let code = unsafe { ((*self.vtbl).get_i)(self, name.as_ptr(), &raw mut out) };
        (!ngx_failed(code)).then_some(out)
    }

    unsafe fn get_ui(&self, name: &CStr) -> Option<u32> {
        let mut out = 0u32;
        // SAFETY: valid vtable slot `0x60` on a live NGX parameter object.
        let code = unsafe { ((*self.vtbl).get_ui)(self, name.as_ptr(), &raw mut out) };
        (!ngx_failed(code)).then_some(out)
    }

    unsafe fn get_ull(&self, name: &CStr) -> Option<u64> {
        let mut out = 0u64;
        // SAFETY: valid vtable slot `0x78` on a live NGX parameter object.
        let code = unsafe { ((*self.vtbl).get_ull)(self, name.as_ptr(), &raw mut out) };
        (!ngx_failed(code)).then_some(out)
    }

    unsafe fn get_void(&self, name: &CStr) -> Option<*mut c_void> {
        let mut out: *mut c_void = null_mut();
        // SAFETY: valid vtable slot `0x40` on a live NGX parameter object.
        let code = unsafe { ((*self.vtbl).get_void_ptr)(self, name.as_ptr(), &raw mut out) };
        (!ngx_failed(code) && !out.is_null()).then_some(out)
    }
}

type InitProjectIdFn = unsafe extern "C" fn(
    *const c_char,
    i32,
    *const c_char,
    *const u16,
    *mut c_void,
    u32,
    *const NvSdkNgxFeatureCommonInfo,
) -> u32;
type InitExtFn = unsafe extern "C" fn(
    u64,
    *const u16,
    *mut c_void,
    u32,
    *const NvSdkNgxFeatureCommonInfo,
) -> u32;
type GetParamsFn = unsafe extern "C" fn(*mut *mut NvSdkNgxParameter) -> u32;
type DestroyParamsFn = unsafe extern "C" fn(*mut NvSdkNgxParameter) -> u32;
type CreateFeatureFn =
    unsafe extern "C" fn(*mut c_void, i32, *mut NvSdkNgxParameter, *mut *mut NvSdkNgxHandle) -> u32;
type ReleaseFeatureFn = unsafe extern "C" fn(*mut NvSdkNgxHandle) -> u32;
type EvaluateFeatureFn = unsafe extern "C" fn(
    *mut c_void,
    *mut NvSdkNgxHandle,
    *const NvSdkNgxParameter,
    *const c_void,
) -> u32;
type NgxCallbackFn = unsafe extern "C" fn(*mut NvSdkNgxParameter) -> u32;

struct CoreLib(HMODULE);

impl Drop for CoreLib {
    fn drop(&mut self) {
        // SAFETY: owns module handle from LoadLibraryExW.
        unsafe {
            let _ = FreeLibrary(self.0);
        }
    }
}

struct NgxRuntime {
    _core: CoreLib,
    device: ID3D12Device,
    dlss_dir: PathBuf,
    _dlss_path_wide: Vec<u16>,
    cap_params: usize,
    allocate_parameters: GetParamsFn,
    destroy_parameters: DestroyParamsFn,
    create_feature: CreateFeatureFn,
    release_feature: ReleaseFeatureFn,
    evaluate_feature: EvaluateFeatureFn,
    cap_lock: Mutex<()>,
}

// SAFETY: NGX runtime entry points and D3D12 device are free-threaded; capability
// parameter queries are serialized by `cap_lock`.
unsafe impl Send for NgxRuntime {}
// SAFETY: as above.
unsafe impl Sync for NgxRuntime {}

static NGX: OnceLock<NgxRuntime> = OnceLock::new();

pub(crate) struct DlssInstance {
    params: usize,
    handle: Option<usize>,
    flags: u32,
    max_render: [u32; 2],
    max_output: [u32; 2],
    created_render: [u32; 2],
    created_output: [u32; 2],
    created_quality: i32,
}

pub(crate) struct DlssEval {
    pub(crate) cmd_list: *mut c_void,
    pub(crate) resources: [*mut c_void; 7],
    pub(crate) jitter: [f32; 2],
    pub(crate) motion_scale: [f32; 2],
    pub(crate) render_size: [u32; 2],
    pub(crate) output_size: [u32; 2],
    pub(crate) settings: [u32; 10],
}

impl DlssInstance {
    pub(crate) fn new(
        flags: u32,
        max_render: [u32; 2],
        max_output: [u32; 2],
    ) -> Result<Self, String> {
        let ngx = NGX
            .get()
            .ok_or_else(|| String::from("NVIDIA DLSS runtime not initialized"))?;
        let mut params: *mut NvSdkNgxParameter = null_mut();
        // SAFETY: live NGX core and valid out pointer.
        let res = unsafe { (ngx.allocate_parameters)(&raw mut params) };
        if ngx_failed(res) || params.is_null() {
            return Err(format!("NVSDK_NGX_D3D12_AllocateParameters failed ({res:#x})"));
        }
        Ok(Self {
            params: params as usize,
            handle: None,
            flags,
            max_render,
            max_output,
            created_render: [0, 0],
            created_output: [0, 0],
            created_quality: -1,
        })
    }

    pub(crate) fn bounds(&self) -> ([u32; 2], [u32; 2], u32) {
        (self.max_render, self.max_output, self.flags)
    }

    pub(crate) fn dispatch(&mut self, eval: &DlssEval) -> Result<(), String> {
        let DlssEval {
            cmd_list,
            resources,
            jitter,
            motion_scale,
            render_size,
            output_size,
            settings,
        } = *eval;
        let ngx = NGX
            .get()
            .ok_or_else(|| String::from("NVIDIA DLSS runtime not initialized"))?;
        let params = self.params as *mut NvSdkNgxParameter;
        let quality = perf_quality_from_sizes(render_size, output_size);
        let need_recreate = self.handle.is_none()
            || self.created_quality != quality
            || self.created_output != output_size
            || render_size[0] > self.created_render[0]
            || render_size[1] > self.created_render[1];

        if need_recreate {
            if let Some(old) = self.handle.take() {
                victory_gpu::external::finish_submitted();
                // SAFETY: all prior GPU work referencing `old` has completed.
                let _ = unsafe { (ngx.release_feature)(old as *mut NvSdkNgxHandle) };
            }
            let create_render = [
                self.max_render[0].max(render_size[0]),
                self.max_render[1].max(render_size[1]),
            ];
            let ngx_flags = translate_create_flags(self.flags);
            let mut handle: *mut NvSdkNgxHandle = null_mut();
            // SAFETY: open D3D12 command list and live NGX parameter block.
            unsafe {
                let p = &mut *params;
                ((*p.vtbl).reset)(p);
                p.set_ui(c"CreationNodeMask", 1);
                p.set_ui(c"VisibilityNodeMask", 1);
                p.set_ui(c"Width", create_render[0]);
                p.set_ui(c"Height", create_render[1]);
                p.set_ui(c"OutWidth", output_size[0]);
                p.set_ui(c"OutHeight", output_size[1]);
                p.set_i(c"PerfQualityValue", quality);
                p.set_i(c"DLSS.Feature.Create.Flags", ngx_flags);
                p.set_i(c"DLSS.Enable.Output.Subrects", 0);
                p.set_i(c"FreeMemOnReleaseFeature", 1);
                apply_render_presets(p);
                let res = (ngx.create_feature)(
                    cmd_list,
                    NVSDK_NGX_FEATURE_SUPER_SAMPLING,
                    params,
                    &raw mut handle,
                );
                if ngx_failed(res) || handle.is_null() {
                    return Err(format!(
                        "NVSDK_NGX_D3D12_CreateFeature failed ({res:#x}) for render={create_render:?} output={output_size:?} quality={quality}"
                    ));
                }
            }
            self.handle = Some(handle as usize);
            self.created_render = create_render;
            self.created_output = output_size;
            self.created_quality = quality;
            tracing::info!(
                render = ?create_render,
                output = ?output_size,
                quality,
                ngx_flags = format_args!("{ngx_flags:#x}"),
                "NVIDIA DLSS feature created on host"
            );
        }

        let handle = self
            .handle
            .ok_or_else(|| String::from("DLSS handle missing"))?
            as *mut NvSdkNgxHandle;
        let enable_sharpening = settings[0] != 0;
        let sharpness = if enable_sharpening {
            f32::from_bits(settings[1]).clamp(0.0, 1.0)
        } else {
            0.0
        };
        let frame_time_delta = {
            let ms = f32::from_bits(settings[2]);
            if ms.is_finite() && ms > 0.0 {
                ms
            } else {
                16.667
            }
        };
        let pre_exposure = {
            let v = f32::from_bits(settings[3]);
            if v.is_finite() && v > 0.0 { v } else { 1.0 }
        };
        let reset = i32::from(settings[4] != 0);
        let mv_scale_x = if motion_scale[0] == 0.0 { 1.0 } else { motion_scale[0] };
        let mv_scale_y = if motion_scale[1] == 0.0 { 1.0 } else { motion_scale[1] };

        // SAFETY: open host command list, live NGX handle and parameters, and live
        // D3D12 resources held for the lifetime of the recording.
        let res = unsafe {
            let p = &mut *params;
            p.set_res(c"Color", resources[0]);
            p.set_res(c"Depth", resources[1]);
            p.set_res(c"MotionVectors", resources[2]);
            p.set_res(c"ExposureTexture", resources[3]);
            p.set_res(c"DLSS.Input.Bias.Current.Color.Mask", resources[4]);
            p.set_res(c"TransparencyMask", resources[5]);
            p.set_res(c"Output", resources[6]);

            p.set_f(c"Jitter.Offset.X", jitter[0]);
            p.set_f(c"Jitter.Offset.Y", jitter[1]);
            p.set_f(c"MV.Scale.X", mv_scale_x);
            p.set_f(c"MV.Scale.Y", mv_scale_y);
            p.set_ui(c"DLSS.Input.Color.Subrect.Base.X", 0);
            p.set_ui(c"DLSS.Input.Color.Subrect.Base.Y", 0);
            p.set_ui(c"DLSS.Input.Depth.Subrect.Base.X", 0);
            p.set_ui(c"DLSS.Input.Depth.Subrect.Base.Y", 0);
            p.set_ui(c"DLSS.Input.MV.Subrect.Base.X", 0);
            p.set_ui(c"DLSS.Input.MV.Subrect.Base.Y", 0);
            p.set_ui(c"DLSS.Input.Translucency.Subrect.Base.X", 0);
            p.set_ui(c"DLSS.Input.Translucency.Subrect.Base.Y", 0);
            p.set_ui(c"DLSS.Input.Bias.Current.Color.Subrect.Base.X", 0);
            p.set_ui(c"DLSS.Input.Bias.Current.Color.Subrect.Base.Y", 0);
            p.set_ui(c"DLSS.Output.Subrect.Base.X", 0);
            p.set_ui(c"DLSS.Output.Subrect.Base.Y", 0);
            p.set_ui(c"DLSS.Render.Subrect.Dimensions.Width", render_size[0]);
            p.set_ui(c"DLSS.Render.Subrect.Dimensions.Height", render_size[1]);
            p.set_f(c"Sharpness", sharpness);
            p.set_i(c"Reset", reset);
            p.set_f(c"DLSS.Pre.Exposure", pre_exposure);
            p.set_f(c"DLSS.Exposure.Scale", 1.0);
            p.set_f(c"FrameTimeDeltaInMsec", frame_time_delta);
            p.set_i(c"DLSS.Indicator.Invert.X.Axis", 0);
            p.set_i(c"DLSS.Indicator.Invert.Y.Axis", 0);

            (ngx.evaluate_feature)(cmd_list, handle, params, null())
        };
        if ngx_failed(res) {
            return Err(format!("NVSDK_NGX_D3D12_EvaluateFeature failed ({res:#x})"));
        }
        Ok(())
    }
}

impl Drop for DlssInstance {
    fn drop(&mut self) {
        let Some(ngx) = NGX.get() else {
            return;
        };
        if let Some(handle) = self.handle.take() {
            victory_gpu::external::finish_submitted();
            // SAFETY: all GPU work referencing this feature has settled.
            unsafe {
                let _ = (ngx.release_feature)(handle as *mut NvSdkNgxHandle);
            }
        }
        // SAFETY: parameter block allocated by `AllocateParameters`.
        unsafe {
            let _ = (ngx.destroy_parameters)(self.params as *mut NvSdkNgxParameter);
        }
    }
}

extern "C" fn ngx_log_callback(message: *const c_char, level: i32, _source: i32) {
    if message.is_null() {
        return;
    }
    // SAFETY: NGX guarantees a NUL-terminated ASCII/UTF-8 message during the callback.
    let msg = unsafe { CStr::from_ptr(message) }.to_string_lossy();
    let trimmed = msg.trim_end();
    if trimmed.is_empty() {
        return;
    }
    if level <= 1 {
        tracing::debug!(target: "victory_hle::dlss::ngx", "{trimmed}");
    } else {
        tracing::trace!(target: "victory_hle::dlss::ngx", "{trimmed}");
    }
}

pub(crate) fn init(explicit_dir: Option<&Path>) -> Result<(), String> {
    if NGX.get().is_some() {
        return Ok(());
    }
    let vendor = victory_gpu::external::adapter_vendor_id();
    if vendor != Some(0x10DE) {
        return Err(format!(
            "NVIDIA DLSS requires an NVIDIA GeForce RTX adapter (0x10DE), found {vendor:#x?}"
        ));
    }
    let dlss_dir = find_dlss_dir(explicit_dir).ok_or_else(|| {
        String::from(
            "nvngx_dlss.dll not found: set VICTORY_DLSS_DIR or [paths].dlss_dir in victory.toml \
             to the folder containing nvngx_dlss.dll (or place nvngx_dlss.dll beside victory.exe)",
        )
    })?;
    let dlss_dir = std::path::absolute(&dlss_dir).unwrap_or(dlss_dir);
    let core_module = load_ngx_core(&dlss_dir)?;
    let library = CoreLib(core_module);

    let sym = |name: &CStr| -> Result<*const c_void, String> {
        // SAFETY: live HMODULE and NUL-terminated symbol name.
        unsafe { GetProcAddress(core_module, name.as_ptr().cast()) }
            .map(|p| p as *const c_void)
            .ok_or_else(|| format!("_nvngx.dll is missing symbol {}", name.to_string_lossy()))
    };

    let device = victory_gpu::external::host_device()
        .ok_or_else(|| String::from("there is no host D3D12 device"))?;

    let app_data = ngx_cache_dir()?;
    let app_data_wide: Vec<u16> = app_data.as_os_str().encode_wide().chain([0]).collect();
    let dlss_path_wide: Vec<u16> = dlss_dir.as_os_str().encode_wide().chain([0]).collect();
    let mut path_ptrs = [dlss_path_wide.as_ptr()];
    let feature_info = NvSdkNgxFeatureCommonInfo {
        path_list_info: NvSdkNgxPathListInfo {
            path: path_ptrs.as_mut_ptr(),
            length: path_ptrs.len() as u32,
        },
        internal_data: null_mut(),
        logging_info: NvSdkNgxLoggingInfo {
            logging_callback: ngx_log_callback as *const c_void,
            minimum_logging_level: 1,
            disable_other_logging_sinks: true,
        },
    };

    let init_res = if let Ok(p) = sym(c"NVSDK_NGX_D3D12_Init_ProjectID") {
        // SAFETY: verified signature from `nvsdk_ngx_d3d12_lib.obj`.
        let init_project_id = unsafe { std::mem::transmute::<*const c_void, InitProjectIdFn>(p) };
        unsafe {
            init_project_id(
                c"a0f57b54-1daf-4934-90ae-c4035c19df04".as_ptr(),
                NVSDK_NGX_ENGINE_TYPE_CUSTOM,
                c"1.0".as_ptr(),
                app_data_wide.as_ptr(),
                device.as_raw(),
                NVSDK_NGX_VERSION_API,
                &raw const feature_info,
            )
        }
    } else {
        let p = sym(c"NVSDK_NGX_D3D12_Init_Ext")?;
        // SAFETY: fallback signature verified from `nvsdk_ngx_d3d12_lib.obj`.
        let init_ext = unsafe { std::mem::transmute::<*const c_void, InitExtFn>(p) };
        unsafe {
            init_ext(
                0x5649_4354_4F52_5936,
                app_data_wide.as_ptr(),
                device.as_raw(),
                NVSDK_NGX_VERSION_API,
                &raw const feature_info,
            )
        }
    };
    if ngx_failed(init_res) {
        return Err(format!(
            "NVIDIA NGX initialization failed ({init_res:#x}); ensure a recent NVIDIA Game Ready Driver is installed"
        ));
    }

    // SAFETY: verified C ABI exports from `_nvngx.dll`.
    let (get_cap, allocate_parameters, destroy_parameters, create_feature, release_feature, evaluate_feature) = unsafe {
        (
            std::mem::transmute::<*const c_void, GetParamsFn>(sym(c"NVSDK_NGX_D3D12_GetCapabilityParameters")?),
            std::mem::transmute::<*const c_void, GetParamsFn>(sym(c"NVSDK_NGX_D3D12_AllocateParameters")?),
            std::mem::transmute::<*const c_void, DestroyParamsFn>(sym(c"NVSDK_NGX_D3D12_DestroyParameters")?),
            std::mem::transmute::<*const c_void, CreateFeatureFn>(sym(c"NVSDK_NGX_D3D12_CreateFeature")?),
            std::mem::transmute::<*const c_void, ReleaseFeatureFn>(sym(c"NVSDK_NGX_D3D12_ReleaseFeature")?),
            std::mem::transmute::<*const c_void, EvaluateFeatureFn>(sym(c"NVSDK_NGX_D3D12_EvaluateFeature")?),
        )
    };

    let mut cap_params: *mut NvSdkNgxParameter = null_mut();
    // SAFETY: NGX initialized above; valid out pointer.
    let cap_res = unsafe { get_cap(&raw mut cap_params) };
    if ngx_failed(cap_res) || cap_params.is_null() {
        return Err(format!(
            "NVSDK_NGX_D3D12_GetCapabilityParameters failed ({cap_res:#x})"
        ));
    }
    // SAFETY: capability parameter object populated by NGX.
    let available = unsafe { (&*cap_params).get_i(c"SuperSampling.Available") }.unwrap_or(0);
    if available == 0 {
        let min_major =
            unsafe { (&*cap_params).get_ui(c"SuperSampling.MinDriverVersionMajor") }.unwrap_or(0);
        let min_minor =
            unsafe { (&*cap_params).get_ui(c"SuperSampling.MinDriverVersionMinor") }.unwrap_or(0);
        let needs_update =
            unsafe { (&*cap_params).get_i(c"SuperSampling.NeedsUpdatedDriver") }.unwrap_or(0);
        let init_status =
            unsafe { (&*cap_params).get_i(c"SuperSampling.FeatureInitResult") }.unwrap_or(0);
        return Err(format!(
            "DLSS Super Resolution unavailable on this GPU/driver (init={init_status:#x}, needs_driver_update={needs_update}, min_driver={min_major}.{min_minor}, dlss_dir={})",
            dlss_dir.display()
        ));
    }

    let runtime = NgxRuntime {
        _core: library,
        device,
        dlss_dir: dlss_dir.clone(),
        _dlss_path_wide: dlss_path_wide,
        cap_params: cap_params as usize,
        allocate_parameters,
        destroy_parameters,
        create_feature,
        release_feature,
        evaluate_feature,
        cap_lock: Mutex::new(()),
    };
    let _ = NGX.set(runtime);
    tracing::info!(
        dlss_dir = %dlss_dir.display(),
        preset = dlss_preset_name(),
        "NVIDIA DLSS initialized on host D3D12 device"
    );
    Ok(())
}

pub(crate) fn dlss_dir() -> Option<&'static Path> {
    NGX.get().map(|n| n.dlss_dir.as_path())
}

fn find_dlss_dir(explicit: Option<&Path>) -> Option<PathBuf> {
    let mut candidates: Vec<PathBuf> = Vec::new();
    if let Some(dir) = explicit {
        candidates.push(dir.to_path_buf());
    }
    if let Some(env) = std::env::var_os("VICTORY_DLSS_DIR")
        && !env.is_empty()
    {
        candidates.push(PathBuf::from(env));
    }
    if let Ok(exe) = std::env::current_exe()
        && let Some(parent) = exe.parent()
    {
        candidates.push(parent.to_path_buf());
        candidates.push(parent.join("dlss"));
    }
    if let Ok(cwd) = std::env::current_dir() {
        candidates.push(cwd.clone());
        candidates.push(cwd.join("dlss"));
    }
    if let Some(env) = std::env::var_os("VICTORY_FIDELITYFX_DIR")
        && !env.is_empty()
    {
        candidates.push(PathBuf::from(env));
    }
    candidates.into_iter().find(|dir| dir.join("nvngx_dlss.dll").is_file())
}

fn load_ngx_core(dlss_dir: &Path) -> Result<HMODULE, String> {
    let mut search_dirs: Vec<PathBuf> = vec![dlss_dir.to_path_buf()];
    if let Some(driver_core) = victory_gpu::external::ngx_core_dir() {
        search_dirs.push(driver_core);
    }
    for dir in &search_dirs {
        for dll in ["_nvngx.dll", "nvngx.dll"] {
            let full = dir.join(dll);
            if !full.is_file() {
                continue;
            }
            let wide: Vec<u16> = full.as_os_str().encode_wide().chain([0]).collect();
            // SAFETY: NUL-terminated path to an existing driver/DLSS NGX core DLL.
            let module = unsafe {
                let m = LoadLibraryExW(
                    wide.as_ptr(),
                    null_mut(),
                    LOAD_LIBRARY_SEARCH_DLL_LOAD_DIR | LOAD_LIBRARY_SEARCH_SYSTEM32,
                );
                if !m.is_null() {
                    m
                } else {
                    LoadLibraryExW(wide.as_ptr(), null_mut(), LOAD_WITH_ALTERED_SEARCH_PATH)
                }
            };
            if !module.is_null() {
                tracing::info!(core = %full.display(), "loaded NVIDIA NGX core");
                return Ok(module);
            }
        }
    }
    Err(String::from(
        "could not locate or load NVIDIA's driver NGX core (_nvngx.dll); reinstall or update the NVIDIA driver",
    ))
}

fn ngx_cache_dir() -> Result<PathBuf, String> {
    let base = std::env::var_os("VICTORY_CACHE_DIR")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var_os("LOCALAPPDATA")
                .filter(|v| !v.is_empty())
                .map(|p| PathBuf::from(p).join("victory").join("cache"))
        })
        .unwrap_or_else(std::env::temp_dir);
    let dir = base.join("ngx");
    std::fs::create_dir_all(&dir).map_err(|e| format!("creating NGX cache dir: {e}"))?;
    Ok(dir)
}

fn dlss_preset_override() -> Option<u32> {
    static PRESET: OnceLock<Option<u32>> = OnceLock::new();
    *PRESET.get_or_init(|| {
        let val = std::env::var("VICTORY_DLSS_PRESET").ok()?;
        let parsed = match val.trim().to_ascii_uppercase().as_str() {
            "DEFAULT" | "0" => Some(PRESET_DEFAULT),
            "E" => Some(PRESET_E),
            "J" => Some(PRESET_J),
            "K" => Some(PRESET_K),
            "L" => Some(PRESET_L),
            "M" => Some(PRESET_M),
            _ => None,
        };
        if parsed.is_none() {
            tracing::warn!(
                preset = val,
                "ignoring unknown VICTORY_DLSS_PRESET; expected default, E, J, K, L, or M"
            );
        }
        parsed
    })
}

fn dlss_preset_name() -> &'static str {
    match dlss_preset_override() {
        Some(PRESET_DEFAULT) => "default",
        Some(PRESET_E) => "E",
        Some(PRESET_J) => "J",
        Some(PRESET_K) => "K",
        Some(PRESET_L) => "L",
        Some(PRESET_M) => "M",
        _ => "K/M (NVIDIA Transformer defaults)",
    }
}

unsafe fn apply_render_presets(p: &mut NvSdkNgxParameter) {
    // SAFETY: valid parameter pointer from caller; sets hint keys matching `nvsdk_ngx_d.lib`.
    unsafe {
        if let Some(preset) = dlss_preset_override() {
            for key in [
                c"DLSS.Hint.Render.Preset.DLAA",
                c"DLSS.Hint.Render.Preset.Quality",
                c"DLSS.Hint.Render.Preset.Balanced",
                c"DLSS.Hint.Render.Preset.Performance",
                c"DLSS.Hint.Render.Preset.UltraPerformance",
                c"DLSS.Hint.Render.Preset.UltraQuality",
            ] {
                p.set_ui(key, preset);
            }
        } else {
            p.set_ui(c"DLSS.Hint.Render.Preset.DLAA", PRESET_UNSPECIFIED_BIT | PRESET_K);
            p.set_ui(c"DLSS.Hint.Render.Preset.Quality", PRESET_UNSPECIFIED_BIT | PRESET_K);
            p.set_ui(c"DLSS.Hint.Render.Preset.Balanced", PRESET_UNSPECIFIED_BIT | PRESET_K);
            p.set_ui(c"DLSS.Hint.Render.Preset.Performance", PRESET_UNSPECIFIED_BIT | PRESET_M);
            p.set_ui(
                c"DLSS.Hint.Render.Preset.UltraPerformance",
                PRESET_UNSPECIFIED_BIT | PRESET_L,
            );
            p.set_ui(
                c"DLSS.Hint.Render.Preset.UltraQuality",
                PRESET_UNSPECIFIED_BIT | PRESET_K,
            );
        }
    }
}

pub(crate) fn translate_create_flags(ffx_flags: u32) -> i32 {
    const FFX_HDR: u32 = 1 << 0;
    const FFX_DISPLAY_MV: u32 = 1 << 1;
    const FFX_MV_JITTERED: u32 = 1 << 2;
    const FFX_DEPTH_INVERTED: u32 = 1 << 3;
    const FFX_AUTO_EXPOSURE: u32 = 1 << 5;

    let mut out = 0i32;
    if ffx_flags & FFX_HDR != 0 {
        out |= NGX_FLAG_IS_HDR;
    }
    if ffx_flags & FFX_DISPLAY_MV == 0 {
        out |= NGX_FLAG_MV_LOW_RES;
    }
    if ffx_flags & FFX_MV_JITTERED != 0 {
        out |= NGX_FLAG_MV_JITTERED;
    }
    if ffx_flags & FFX_DEPTH_INVERTED != 0 {
        out |= NGX_FLAG_DEPTH_INVERTED;
    }
    if ffx_flags & FFX_AUTO_EXPOSURE != 0 {
        out |= NGX_FLAG_AUTO_EXPOSURE;
    }
    out
}

pub(crate) fn ffx_quality_ratio(quality: u32) -> Option<f32> {
    Some(match quality {
        0 => 1.0, // NativeAA / DLAA
        1 => 1.5, // Quality
        2 => 1.7, // Balanced
        3 => 2.0, // Performance
        4 => 3.0, // UltraPerformance
        _ => return None,
    })
}

fn ffx_quality_to_ngx(quality: u32) -> i32 {
    match quality {
        0 => PERF_DLAA,
        1 => PERF_MAX_QUALITY,
        2 => PERF_BALANCED,
        4 => PERF_ULTRA_PERF,
        _ => PERF_MAX_PERF,
    }
}

fn perf_quality_from_sizes(render: [u32; 2], output: [u32; 2]) -> i32 {
    let ratio = output[0] as f32 / render[0].max(1) as f32;
    if ratio <= 1.05 {
        PERF_DLAA
    } else if ratio <= 1.60 {
        PERF_MAX_QUALITY
    } else if ratio <= 1.85 {
        PERF_BALANCED
    } else if ratio <= 2.50 {
        PERF_MAX_PERF
    } else {
        PERF_ULTRA_PERF
    }
}

pub(crate) fn query_optimal_resolution(display: [u32; 2], quality: u32) -> Option<[u32; 2]> {
    let ratio = ffx_quality_ratio(quality)?;
    let fallback = display.map(|d| ((d as f32 / ratio).round() as u32).max(1));
    let Some(ngx) = NGX.get() else {
        return Some(fallback);
    };
    let _guard = ngx.cap_lock.lock().unwrap_or_else(|e| e.into_inner());
    let params = ngx.cap_params as *mut NvSdkNgxParameter;
    // SAFETY: serialized by `cap_lock`; calls NGX capability optimal-settings callback.
    unsafe {
        let p = &mut *params;
        if let Some(cb) = p.get_void(c"DLSSOptimalSettingsCallback") {
            p.set_ui(c"Width", display[0]);
            p.set_ui(c"Height", display[1]);
            p.set_i(c"PerfQualityValue", ffx_quality_to_ngx(quality));
            p.set_i(c"RTXValue", 0);
            let cb_fn = std::mem::transmute::<*mut c_void, NgxCallbackFn>(cb);
            if !ngx_failed(cb_fn(p))
                && let (Some(w), Some(h)) = (p.get_ui(c"OutWidth"), p.get_ui(c"OutHeight"))
                && w > 0
                && h > 0
            {
                return Some([w, h]);
            }
        }
    }
    Some(fallback)
}

pub(crate) fn query_memory_usage() -> [u64; 2] {
    let Some(ngx) = NGX.get() else {
        return [64 << 20, 64 << 20];
    };
    let _guard = ngx.cap_lock.lock().unwrap_or_else(|e| e.into_inner());
    let params = ngx.cap_params as *mut NvSdkNgxParameter;
    // SAFETY: serialized by `cap_lock`; calls NGX stats callback if present.
    unsafe {
        let p = &mut *params;
        if let Some(cb) = p.get_void(c"DLSSGetStatsCallback") {
            let cb_fn = std::mem::transmute::<*mut c_void, NgxCallbackFn>(cb);
            if !ngx_failed(cb_fn(p))
                && let Some(bytes) = p.get_ull(c"SizeInBytes")
                && bytes > 0
            {
                return [bytes, bytes];
            }
        }
    }
    [64 << 20, 64 << 20]
}

pub(crate) fn jitter_phase_count(render_width: u32, output_width: u32) -> i32 {
    if render_width == 0 || output_width == 0 {
        return 8;
    }
    let ratio = output_width as f32 / render_width as f32;
    (8.0 * ratio * ratio).ceil().clamp(8.0, 64.0) as i32
}

pub(crate) fn jitter_offset(index: i32, phases: i32) -> [f32; 2] {
    let phases = phases.max(1);
    let wrapped = ((index % phases) + phases) % phases + 1;
    [halton(wrapped, 2) - 0.5, halton(wrapped, 3) - 0.5]
}

fn halton(mut index: i32, base: i32) -> f32 {
    let mut f = 1.0f32;
    let mut r = 0.0f32;
    let inv_base = 1.0 / base as f32;
    while index > 0 {
        f *= inv_base;
        r += f * (index % base) as f32;
        index /= base;
    }
    r
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn translates_ffx_flags_and_calculates_safe_jitter() {
        // Title's typical flags 0x29 = HDR | DepthInverted | AutoExposure (low-res MVs).
        assert_eq!(
            translate_create_flags(0x29),
            NGX_FLAG_IS_HDR | NGX_FLAG_MV_LOW_RES | NGX_FLAG_DEPTH_INVERTED | NGX_FLAG_AUTO_EXPOSURE
        );
        // With DisplayResolutionMotionVectors bit (0x2) set, MVLowRes is omitted.
        assert_eq!(
            translate_create_flags(0x2b),
            NGX_FLAG_IS_HDR | NGX_FLAG_DEPTH_INVERTED | NGX_FLAG_AUTO_EXPOSURE
        );
        assert!(jitter_phase_count(960, 1920) >= 8);
        assert!(jitter_phase_count(1280, 3840) >= 8);
        assert!(jitter_phase_count(0, 1920) >= 8);
        let [jx, jy] = jitter_offset(0, jitter_phase_count(1280, 1920));
        assert!(jx.abs() <= 0.5 && jy.abs() <= 0.5);
    }
}
