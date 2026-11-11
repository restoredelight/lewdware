use std::path::PathBuf;

use anyhow::{Context, Result};
use ffmpeg_next::{Packet, Rational, frame};
use serde::{Deserialize, Serialize};

#[cfg(target_os = "windows")]
use super::paths::new_command;
use super::{
    av::{self, Graph, HwDevice, Pull, VideoCodec},
    worker,
};

/// The quality every video encode aims for, on SVT-AV1's CRF scale (0-63).
///
/// Measured on real-world 720p H.264 sources at preset 8: CRF 31 came out at 1.01x x264's
/// (`-crf 23`) size with VMAF 97.0 against x264's 96.7, and CRF 35 at 0.76x the size with VMAF
/// 96.1, both in ~0.8x x264's time. 33 sits between: smaller than the H.264 it replaces, at no
/// visible cost.
pub(super) const SOFTWARE_CRF: u32 = 33;

/// The same target on the AV1 quantizer-index scale (0-255) the hardware encoders take. SVT-AV1
/// maps CRF to a base qindex of roughly four times its value.
const HARDWARE_QINDEX: u32 = SOFTWARE_CRF * 4;

/// SVT-AV1's speed/efficiency trade-off, 0 (slowest) to 13. 8 is where it overtakes x264's
/// `medium` in speed while still beating it on size.
const SOFTWARE_PRESET: u32 = 8;

/// Which AV1 encoder a video goes through.
///
/// Hardware AV1 encoding is newer than hardware decoding -- Nvidia from Ada (RTX 40), AMD from
/// RDNA 3 (RX 7000), Intel from Arc and Meteor Lake -- so on most machines this ends up
/// `SoftwareFallback`, which is SVT-AV1. Apple Silicon has no AV1 encoder at all.
///
/// The hardware settings below are untested on real hardware: `test` proves an encoder opens and
/// produces packets on this machine, not that its quality matches `SOFTWARE_CRF`.
#[derive(Clone, Serialize, Deserialize, Debug, PartialEq, Eq)]
pub enum HardwareEncoder {
    Nvidia,
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
    /// candidate can fail for reasons no amount of inspection would reveal -- a GPU generation
    /// without an AV1 encoder, a driver package that is not installed, a render node belonging to
    /// a device with no encode engine at all (`vgem` and `vkms` both create one), a sandbox that
    /// cannot reach the hardware -- and on a machine with more than one GPU the answer differs
    /// *between* devices, which is why the VAAPI candidates are per render node rather than one
    /// guess at which node is the right one.
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
            let mut candidates = Vec::new();
            if let Ok(output) = new_command("powershell")
                .args(["-Command", "(Get-CimInstance Win32_VideoController).Name"])
                .output()
            {
                let gpu_name = String::from_utf8_lossy(&output.stdout).to_lowercase();
                if gpu_name.contains("nvidia") {
                    candidates.push(Self::Nvidia);
                }
                if gpu_name.contains("amd") || gpu_name.contains("radeon") {
                    candidates.push(Self::Amd);
                }
                if gpu_name.contains("intel") {
                    candidates.push(Self::Intel);
                }
            }
            return candidates;
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
            if let Ok(output) = super::paths::new_command("lspci").output()
                && String::from_utf8_lossy(&output.stdout)
                    .to_lowercase()
                    .contains("nvidia")
            {
                candidates.push(Self::Nvidia);
            }
            candidates.extend(vaapi_render_nodes().into_iter().map(Self::Vaapi));
            return candidates;
        }

        // macOS: VideoToolbox has no AV1 encoder, on any Apple Silicon generation so far.
        #[allow(unreachable_code)]
        Vec::new()
    }

    /// The device frames have to be uploaded to first, for an encoder that only takes frames
    /// already in GPU memory.
    pub(super) fn device(&self) -> Result<Option<HwDevice>> {
        match self {
            Self::Vaapi(node) => HwDevice::vaapi(node).map(Some),
            _ => Ok(None),
        }
    }

    /// How the encoder wants its frames handed over: the tail of the filter chain feeding it.
    ///
    /// Every encoder but VAAPI takes ordinary software frames, so this is just the pixel format.
    /// VAAPI encodes from a surface in GPU memory, so its frames are also uploaded onto the device
    /// from `device`, which `av::Graph` adds after this.
    pub(super) fn input_filter(&self) -> &'static str {
        match self {
            Self::Vaapi(_) | Self::Intel | Self::Amd => "format=nv12",
            Self::Nvidia | Self::SoftwareFallback => "format=yuv420p",
        }
    }

    pub(super) fn codec(&self) -> VideoCodec {
        let q = HARDWARE_QINDEX.to_string();
        let (name, options): (_, Vec<(&'static str, String)>) = match self {
            // NVENC's AV1 `cq` is on the CRF-like 0-63 scale, not the qindex one.
            Self::Nvidia => (
                "av1_nvenc",
                vec![
                    ("preset", "p4".into()),
                    ("rc", "vbr".into()),
                    ("cq", SOFTWARE_CRF.to_string()),
                    ("b", "0".into()),
                ],
            ),
            Self::Intel => ("av1_qsv", vec![("global_quality", q)]),
            Self::Amd => (
                "av1_amf",
                vec![
                    ("usage", "transcoding".into()),
                    ("quality", "quality".into()),
                    ("rc", "cqp".into()),
                    ("qp_i", q.clone()),
                    ("qp_p", q),
                ],
            ),
            Self::Vaapi(_) => (
                "av1_vaapi",
                vec![("rc_mode", "CQP".into()), ("global_quality", q)],
            ),
            Self::SoftwareFallback => (
                "libsvtav1",
                vec![
                    ("preset", SOFTWARE_PRESET.to_string()),
                    ("crf", SOFTWARE_CRF.to_string()),
                ],
            ),
        };
        VideoCodec {
            name,
            options,
            range: None,
        }
    }

    /// Encodes a few black frames to find out whether this machine can really do what
    /// `candidates` guessed from the hardware it named. A present GPU is not a working encoder:
    /// the generation can lack AV1 encode, the driver can be missing (`av1_qsv` without oneVPL),
    /// the runtime unavailable (`av1_amf` outside AMDGPU-PRO), or the device unreachable from a
    /// sandbox.
    ///
    /// Built from the same pieces `encode_video` uses -- device, input filter, encoder -- so a pass
    /// here means that pipeline works, not merely that the codec exists. Runs in the encode
    /// worker when there is one, since a misbehaving driver is as likely to crash as to fail.
    pub fn test(self) -> Self {
        if self == Self::SoftwareFallback {
            return self;
        }
        let ok = match worker::configured() {
            Some(exe) => worker::test(exe, &self),
            None => self.test_in_process(),
        };
        if ok { self } else { Self::SoftwareFallback }
    }

    pub(super) fn test_in_process(&self) -> bool {
        match self.try_encode_test_frames() {
            Ok(()) => true,
            Err(error) => {
                tracing::info!("{self:?} can't encode here: {error:#}");
                false
            }
        }
    }

    fn try_encode_test_frames(&self) -> Result<()> {
        let device = self.device()?;
        // 256x256: some hardware AV1 encoders refuse anything under 128 on a side.
        let spec = format!(
            "color=c=black:s=256x256:r=30:d=0.2,{}[main]",
            self.input_filter()
        );
        let mut graph = Graph::new(None, &spec, &["main"], device.as_ref())?;

        let mut frame = frame::Video::empty();
        let mut packet = Packet::empty();
        let mut encoder = None;
        let mut packets = 0;
        while let Pull::Ready = graph.pull("main", &mut frame)? {
            let encoder = match &mut encoder {
                Some(encoder) => encoder,
                None => encoder.insert(av::open_video_encoder(
                    &mut graph,
                    "main",
                    &frame,
                    &self.codec(),
                    Rational::new(30, 1),
                    false,
                )?),
            };
            encoder.send_frame(&frame)?;
            while let Pull::Ready = av::receive_packet(encoder, &mut packet)? {
                packets += 1;
            }
        }
        let mut encoder = encoder.context("the test source made no frames")?;
        encoder.send_eof()?;
        while let Pull::Ready = av::receive_packet(&mut encoder, &mut packet)? {
            packets += 1;
        }
        anyhow::ensure!(packets > 0, "the encoder produced no packets");
        Ok(())
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
