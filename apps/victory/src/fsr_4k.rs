//! PC upscaler profiles (NVIDIA DLSS and AMD FidelityFX FSR 4.1.2 / 3.1)
//! across all title display resolutions.
use anyhow::{Context, Result, ensure};
use victory_loader::PreparedImage;

const RETAIL_BUILD: &str = "b99f876b74d4a77f03af5f878223b27bd647b5c35a79d5decd9f716504239e93";
const SDR_MODE: u32 = 0x0615_c100;
const STOCK_SDR_MODE: [u8; 16] = [0, 15, 0, 0, 112, 8, 0, 0, 60, 0, 0, 0, 1, 0, 0, 0];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum UpscalerBackend {
    FidelityFx,
    Dlss,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum FsrPreset {
    Native,
    Quality,
    Balanced,
    Performance,
    UltraPerformance,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct UpscalerSelection {
    pub(super) backend: UpscalerBackend,
    pub(super) preset: FsrPreset,
    pub(super) from_fsr_4k: bool,
}

impl UpscalerSelection {
    pub(super) fn requested() -> Result<Option<Self>> {
        if let Some(preset) = parse_env("VICTORY_DLSS", true)? {
            return Ok(Some(Self {
                backend: UpscalerBackend::Dlss,
                preset,
                from_fsr_4k: false,
            }));
        }
        if let Some(preset) = parse_env("VICTORY_FSR", true)? {
            return Ok(Some(Self {
                backend: UpscalerBackend::FidelityFx,
                preset,
                from_fsr_4k: false,
            }));
        }
        if let Some(preset) = FsrPreset::requested()? {
            return Ok(Some(Self {
                backend: UpscalerBackend::FidelityFx,
                preset,
                from_fsr_4k: true,
            }));
        }
        Ok(None)
    }
}

fn parse_env(var: &str, extended: bool) -> Result<Option<FsrPreset>> {
    match std::env::var(var) {
        Err(std::env::VarError::NotPresent) => Ok(None),
        Ok(raw) => {
            let norm = raw.trim().to_ascii_lowercase();
            if matches!(norm.as_str(), "" | "0" | "off" | "false") {
                return Ok(None);
            }
            if extended && matches!(norm.as_str(), "1" | "auto" | "on" | "true") {
                return Ok(Some(FsrPreset::Quality));
            }
            let parsed = if extended {
                FsrPreset::from_extended_name(&norm)
            } else {
                FsrPreset::from_name(&norm)
            };
            parsed.map(Some).with_context(|| {
                if extended {
                    format!(
                        "{var}={raw}: expected dlaa/native, quality, balanced, performance, ultra_performance, or 0"
                    )
                } else {
                    format!("{var}={raw}: expected quality, balanced, performance or 0")
                }
            })
        }
        Err(error) => Err(error.into()),
    }
}

impl FsrPreset {
    pub(super) fn requested() -> Result<Option<Self>> {
        parse_env("VICTORY_FSR_4K", false)
    }

    pub(super) fn from_name(name: &str) -> Option<Self> {
        match name {
            "quality" => Some(Self::Quality),
            "balanced" => Some(Self::Balanced),
            "performance" => Some(Self::Performance),
            _ => None,
        }
    }

    pub(super) fn from_extended_name(name: &str) -> Option<Self> {
        match name {
            "dlaa" | "native" | "native_aa" | "nativeaa" => Some(Self::Native),
            "quality" => Some(Self::Quality),
            "balanced" => Some(Self::Balanced),
            "performance" | "perf" => Some(Self::Performance),
            "ultra_performance" | "ultra-performance" | "ultraperformance" | "ultra" => {
                Some(Self::UltraPerformance)
            }
            _ => None,
        }
    }

    pub(super) fn name(self) -> &'static str {
        match self {
            Self::Native => "native",
            Self::Quality => "quality",
            Self::Balanced => "balanced",
            Self::Performance => "performance",
            Self::UltraPerformance => "ultra-performance",
        }
    }

    // FFX_UPSCALE_QUALITY_MODE values and ratios.
    pub(super) fn mode(self) -> u8 {
        match self {
            Self::Native => 0,
            Self::Quality => 1,
            Self::Balanced => 2,
            Self::Performance => 3,
            Self::UltraPerformance => 4,
        }
    }

    pub(super) fn render_scale(self) -> f32 {
        match self {
            Self::Native => 1.0,
            Self::Quality => 2.0 / 3.0,
            Self::Balanced => 1.0 / 1.7,
            Self::Performance => 0.5,
            Self::UltraPerformance => 1.0 / 3.0,
        }
    }

    pub(super) fn render_size_for(self, output: [u32; 2]) -> [u32; 2] {
        output.map(|size| ((size as f32 * self.render_scale()) as u32).max(1))
    }

    #[cfg(test)]
    pub(super) fn render_size(self) -> [u32; 2] {
        self.render_size_for([3840, 2160])
    }
}

#[cfg(test)]
pub(super) fn prepare_fsr_4k(image: &mut PreparedImage, preset: FsrPreset) -> Result<()> {
    prepare_upscaler(image, preset, "FSR", [3840, 2160])
}

pub(super) fn prepare_upscaler(
    image: &mut PreparedImage,
    preset: FsrPreset,
    backend_label: &str,
    output: [u32; 2],
) -> Result<()> {
    ensure!(
        image.original_hash.to_hex().as_str() == RETAIL_BUILD,
        "{backend_label} upscaling patch is unavailable for this retail game build"
    );
    ensure!(
        *field::<16>(image, SDR_MODE)? == STOCK_SDR_MODE,
        "Unexpected Series X SDR mode"
    );
    ensure!(
        *field::<4>(image, 0x0615_d95c)? == (2.0f32 / 3.0).to_le_bytes(),
        "Unexpected Series X FSR scene scale"
    );
    ensure!(
        *field::<8>(image, 0x007e_9421)? == [0xb8, 6, 0, 0, 0, 0x0f, 0x45, 0xc1],
        "Unexpected Series X temporal renderer selection"
    );
    ensure!(
        *field::<14>(image, 0x028e_f69a)?
            == [
                0x0f, 0xb6, 0xc0, 0x41, 0x89, 0x86, 0x90, 0, 0, 0, 0x84, 0xc0, 0x75, 0x0a
            ],
        "Unexpected Series X FSR render size selection"
    );
    const NATIVE_LOAD: u32 = 0x028e_f6a8;
    const QUALITY_LOAD: u32 = 0x028e_f6b2;
    if !matches!(preset, FsrPreset::Native | FsrPreset::Quality) {
        ensure!(
            *field::<10>(image, NATIVE_LOAD)?
                == [0xc5, 0xfa, 0x10, 0x05, 0x28, 0x78, 0x6d, 0x03, 0xeb, 0x08]
                && *field::<8>(image, QUALITY_LOAD)?
                    == [0xc5, 0xfa, 0x10, 0x05, 0xa2, 0xe2, 0x86, 0x03],
            "Unexpected Series X FSR scale instructions"
        );
    }
    // The original selector has already run its capability check. An
    // explicit PC upscaler session uses renderer 7 every time it queries this.
    *field::<8>(image, 0x007e_9421)? = [0xb8, 7, 0, 0, 0, 0x90, 0x90, 0x90];
    if preset == FsrPreset::Native {
        // Record quality mode 0 (Native / DLAA) and fall through to NATIVE_LOAD (1.0x).
        *field::<14>(image, 0x028e_f69a)? = [
            0x31, 0xc0, 0x41, 0x89, 0x86, 0x90, 0, 0, 0, 0x90, 0x90, 0x90, 0x90, 0x90,
        ];
    } else {
        // Record the SDK mode and jump to the scaled-scene branch.
        *field::<14>(image, 0x028e_f69a)? = [
            0xb8,
            preset.mode(),
            0,
            0,
            0,
            0x41,
            0x89,
            0x86,
            0x90,
            0,
            0,
            0,
            0xeb,
            0x0a,
        ];
        if preset != FsrPreset::Quality {
            // The jump makes the native-scale instruction unreachable. Reuse
            // its RIP operand bytes for a private literal, leaving the constants
            // used by other passes unchanged.
            let local_scale = NATIVE_LOAD + 4;
            *field::<4>(image, local_scale)? = preset.render_scale().to_le_bytes();
            let displacement = local_scale as i32 - (QUALITY_LOAD as i32 + 8);
            *field::<4>(image, QUALITY_LOAD + 4)? = displacement.to_le_bytes();
        }
    }
    tracing::info!(
        renderer = 7,
        backend = backend_label,
        preset = preset.name(),
        quality = preset.mode(),
        expected_render = ?preset.render_size_for(output),
        ?output,
        "Selected PC temporal upscaler profile"
    );
    Ok(())
}

fn field<const N: usize>(image: &mut PreparedImage, rva: u32) -> Result<&mut [u8; N]> {
    let offset = image
        .info
        .rva_to_file_offset(rva)
        .context("Display profile field missing")? as usize;
    image
        .bytes
        .get_mut(offset..offset + N)
        .context("Incomplete display profile field")?
        .try_into()
        .context("Invalid display profile field length")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_extended_upscaler_presets() {
        assert_eq!(FsrPreset::from_extended_name("dlaa"), Some(FsrPreset::Native));
        assert_eq!(FsrPreset::from_extended_name("native"), Some(FsrPreset::Native));
        assert_eq!(FsrPreset::from_extended_name("quality"), Some(FsrPreset::Quality));
        assert_eq!(FsrPreset::from_extended_name("balanced"), Some(FsrPreset::Balanced));
        assert_eq!(
            FsrPreset::from_extended_name("performance"),
            Some(FsrPreset::Performance)
        );
        assert_eq!(
            FsrPreset::from_extended_name("ultra_performance"),
            Some(FsrPreset::UltraPerformance)
        );
        assert_eq!(FsrPreset::Native.render_size_for([1920, 1080]), [1920, 1080]);
        assert_eq!(FsrPreset::Quality.render_size_for([1920, 1080]), [1280, 720]);
        assert_eq!(
            FsrPreset::Performance.render_size_for([1920, 1080]),
            [960, 540]
        );
        assert_eq!(
            FsrPreset::Quality.render_size_for([2560, 1440]),
            [1706, 960]
        );
    }

    #[test]
    fn fsr_presets_leave_the_stock_scanout_and_settings_viewport() {
        let Some(path) = std::env::var_os("VICTORY_DISPLAY_PROFILE_TEST_IMAGE") else {
            return;
        };
        let bytes = std::fs::read(path).unwrap();
        for (preset, size) in [
            (FsrPreset::Quality, [2560, 1440]),
            (FsrPreset::Balanced, [2258, 1270]),
            (FsrPreset::Performance, [1920, 1080]),
        ] {
            let mut image = victory_loader::prepare(&bytes).unwrap();
            let scanout = *field::<5>(&mut image, 0x0247_ad4a).unwrap();
            let settings = *field::<8>(&mut image, 0x0580_48d4).unwrap();
            prepare_fsr_4k(&mut image, preset).unwrap();
            assert_eq!(*field::<16>(&mut image, SDR_MODE).unwrap(), STOCK_SDR_MODE);
            assert_eq!(*field::<5>(&mut image, 0x0247_ad4a).unwrap(), scanout);
            assert_eq!(*field::<8>(&mut image, 0x0580_48d4).unwrap(), settings);
            assert_eq!(
                *field::<4>(&mut image, 0x007e_9422).unwrap(),
                7u32.to_le_bytes()
            );
            assert_eq!(
                *field::<4>(&mut image, 0x028e_f69b).unwrap(),
                u32::from(preset.mode()).to_le_bytes()
            );
            assert_eq!(preset.render_size(), size);
            let disp = i32::from_le_bytes(*field::<4>(&mut image, 0x028e_f6b6).unwrap());
            let reference = 0x028e_f6ba_u32.wrapping_add_signed(disp);
            let scale = f32::from_le_bytes(*field::<4>(&mut image, reference).unwrap());
            assert_eq!([3840.0, 2160.0].map(|w| (w * scale) as u32), size);
        }
    }
}
