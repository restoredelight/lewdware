use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use super::paths::{get_ffmpeg_path, new_command};

#[derive(Clone, Serialize, Deserialize, Debug, PartialEq, Eq)]
pub enum HardwareEncoder {
    Nvidia,
    Apple,
    Intel,
    Amd,
    /// Not used for Nvidia even though `nvidia-vaapi-driver` exists: that driver is decode-only,
    /// and NVENC is the better encoder there anyway.
    ///
    /// Carries the DRM render node to encode on, because on a multi-GPU machine the encoder and
    /// the device are one choice, not two: `detect_and_test` settles them together.
    Vaapi(PathBuf),
    SoftwareFallback,
}

impl HardwareEncoder {
    /// Picks the encoder this machine can really use: the first candidate that encodes a frame.
    ///
    /// Naming the GPU only narrows the field, so nothing here is trusted until it has run. A
    /// candidate can fail for reasons no amount of inspection would reveal -- a driver package
    /// that is not installed, a render node belonging to a device with no encode engine at all
    /// (`vgem` and `vkms` both create one), a sandbox that cannot reach the hardware -- and on a
    /// machine with more than one GPU the answer differs *between* devices, which is why the VAAPI
    /// candidates are per render node rather than one guess at which node is the right one.
    pub fn detect_and_test() -> Self {
        Self::candidates()
            .into_iter()
            .map(Self::test)
            .find(|tested| *tested != Self::SoftwareFallback)
            .unwrap_or(Self::SoftwareFallback)
    }

    /// Every encoder worth trying here, best first.
    fn candidates() -> Vec<Self> {
        #[cfg(target_os = "windows")]
        {
            if let Ok(output) = new_command("powershell")
                .args(["-Command", "(Get-CimInstance Win32_VideoController).Name"])
                .output()
            {
                let gpu_name = String::from_utf8_lossy(&output.stdout).to_lowercase();
                if gpu_name.contains("nvidia") {
                    return vec![Self::Nvidia];
                }
                if gpu_name.contains("amd") || gpu_name.contains("radeon") {
                    return vec![Self::Amd];
                }
                if gpu_name.contains("intel") {
                    return vec![Self::Intel];
                }
            }
        }

        #[cfg(target_os = "macos")]
        {
            return vec![Self::Apple];
        }

        // Nvidia is the only vendor worth naming on Linux: NVENC beats VAAPI there, and
        // `nvidia-vaapi-driver` cannot encode at all. Everything else -- Intel, AMD, and the
        // virtualised GPUs that answer to neither name -- goes through VAAPI.
        //
        // The two are tried in turn rather than either/or, because a machine can offer both. On a
        // hybrid laptop `lspci` reports the Nvidia chip whether or not its proprietary driver is
        // installed; without it NVENC fails, and falling through to the Intel iGPU's VAAPI beats
        // dropping all the way to software.
        #[cfg(target_os = "linux")]
        {
            let mut candidates = Vec::new();
            if let Ok(output) = new_command("lspci").output()
                && String::from_utf8_lossy(&output.stdout)
                    .to_lowercase()
                    .contains("nvidia")
            {
                candidates.push(Self::Nvidia);
            }
            candidates.extend(vaapi_render_nodes().into_iter().map(Self::Vaapi));
            return candidates;
        }

        // If all checks fail, fallback to safe CPU encoding
        #[allow(unreachable_code)]
        Vec::new()
    }

    /// Global options that have to precede the input, because they set up the hardware device the
    /// filter graph and the encoder then refer to by name.
    pub fn init_args(&self) -> Vec<String> {
        match self {
            Self::Vaapi(node) => vec![
                "-init_hw_device".into(),
                format!("vaapi=va:{}", node.display()),
                "-filter_hw_device".into(),
                "va".into(),
            ],
            _ => Vec::new(),
        }
    }

    /// How the encoder wants its frames handed over: the tail of the filter chain feeding it.
    ///
    /// Every encoder but VAAPI takes ordinary software frames, so this is just the pixel format.
    /// VAAPI encodes from a surface in GPU memory, so the chain has to convert to the format the
    /// hardware accepts and then `hwupload` it onto the device `init_args` opened.
    pub fn input_filter(&self) -> &'static str {
        match self {
            Self::Vaapi(_) => "format=nv12,hwupload",
            _ => "format=yuv420p",
        }
    }

    pub fn ffmpeg_args(&self) -> &[&'static str] {
        match self {
            Self::Nvidia => &[
                "-c:v",
                "h264_nvenc",
                "-preset",
                "p4",
                "-cq",
                "23",
                "-b:v",
                "0",
            ],
            Self::Apple => &["-c:v", "h264_videotoolbox", "-q:v", "60"],
            Self::Intel => &["-c:v", "h264_qsv", "-global_quality:v", "23"],
            Self::Amd => &[
                "-c:v", "h264_amf", "-quality", "quality", "-rc", "cqp", "-qp_i", "23", "-qp_p",
                "23", "-qp_b", "23",
            ],
            Self::Vaapi(_) => &["-c:v", "h264_vaapi", "-rc_mode", "CQP", "-qp", "23"],
            Self::SoftwareFallback => &["-c:v", "libx264", "-crf", "23"],
        }
    }

    /// Encodes one black frame to find out whether this machine can really do what `detect()`
    /// guessed from the hardware it named. A present GPU is not a working encoder: the driver can
    /// be missing (`h264_qsv` without oneVPL), the runtime unavailable (`h264_amf` outside
    /// AMDGPU-PRO), or the device unreachable from a sandbox.
    ///
    /// Built the same way `encode_video` builds the real thing -- device setup, input filter,
    /// codec -- so that a pass here means that command shape works, not merely that the codec
    /// exists.
    pub fn test(self) -> Self {
        if self == Self::SoftwareFallback {
            return self;
        }

        let ok = new_command(get_ffmpeg_path())
            .args(self.init_args())
            .args([
                "-f",
                "lavfi",
                "-i",
                "color=c=black:s=128x128",
                "-vframes",
                "1",
            ])
            .args(["-vf", self.input_filter()])
            .args(self.ffmpeg_args())
            .args(["-f", "null", "-"])
            .status()
            .is_ok_and(|status| status.success());

        if ok { self } else { Self::SoftwareFallback }
    }
}

/// Every DRM render node on this machine, lowest-numbered first.
///
/// Render nodes are the unprivileged half of a DRM device -- no display access, just compute and
/// media -- so any user can open one, and a Flatpak gets them from `--device=dri`. They are
/// numbered from `renderD128` upwards; the numbering follows driver load order, so it says nothing
/// about which device can encode, or even whether a node belongs to real hardware. Hence all of
/// them, for `detect_and_test` to try in turn.
#[cfg(target_os = "linux")]
fn vaapi_render_nodes() -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir("/dev/dri") else {
        return Vec::new();
    };
    let mut nodes: Vec<PathBuf> = entries
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("renderD"))
        })
        .collect();
    // Fixed-width names in a fixed range, so ordering them as text orders them as numbers.
    nodes.sort();
    nodes
}
