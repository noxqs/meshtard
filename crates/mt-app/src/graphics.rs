//! Preferences > Graphics: GPU selection (applies after a restart) and Show Log File
//! (a graphics log written at start-up to the user data folder); Preferences >
//! Applications: the external image editor (FEATURES.md 43). SketchUp's help lists
//! the GPU choice "when several are installed" and the log file button; the wording
//! and what the log holds here are meshtard's own (unverified).

use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};

use egui_wgpu::wgpu;

/// The label an adapter is listed and saved under: its name and backend.
pub(crate) fn adapter_label(info: &wgpu::AdapterInfo) -> String {
    format!("{} ({:?})", info.name, info.backend)
}

/// Rank of an adapter type when nothing is chosen (discrete first).
fn rank(t: wgpu::DeviceType) -> u8 {
    match t {
        wgpu::DeviceType::DiscreteGpu => 0,
        wgpu::DeviceType::IntegratedGpu => 1,
        wgpu::DeviceType::VirtualGpu => 2,
        wgpu::DeviceType::Other => 3,
        wgpu::DeviceType::Cpu => 4,
    }
}

/// The adapter to use among `labels` (with their types): the one labelled `wanted`,
/// else the best ranked. `None` for an empty list.
pub(crate) fn choose(
    adapters: &[(String, wgpu::DeviceType)],
    wanted: Option<&str>,
) -> Option<usize> {
    if let Some(i) = wanted.and_then(|w| adapters.iter().position(|(l, _)| l == w)) {
        return Some(i);
    }
    (0..adapters.len()).min_by_key(|&i| rank(adapters[i].1))
}

/// The GPU saved in `preferences.toml` (`[graphics] gpu`), read before the window
/// opens.
pub(crate) fn saved_gpu() -> Option<String> {
    let path = crate::settings::config_dir()?.join("preferences.toml");
    let table: toml::Table = std::fs::read_to_string(path).ok()?.parse().ok()?;
    let gpu = table.get("graphics")?.get("gpu")?.as_str()?;
    (!gpu.is_empty()).then(|| gpu.to_owned())
}

/// The renderer options: with a saved GPU, an adapter selector that takes it when
/// present (and the usual choice otherwise).
pub(crate) fn wgpu_options(gpu: Option<String>) -> egui_wgpu::WgpuConfiguration {
    let mut c = egui_wgpu::WgpuConfiguration::default();
    if let (Some(gpu), egui_wgpu::WgpuSetup::CreateNew(new)) = (gpu, &mut c.wgpu_setup) {
        new.native_adapter_selector = Some(Arc::new(move |adapters, surface| {
            let usable: Vec<&wgpu::Adapter> = adapters
                .iter()
                .filter(|a| surface.is_none_or(|s| a.is_surface_supported(s)))
                .collect();
            let list: Vec<(String, wgpu::DeviceType)> = usable
                .iter()
                .map(|a| {
                    let info = a.get_info();
                    (adapter_label(&info), info.device_type)
                })
                .collect();
            choose(&list, Some(&gpu))
                .map(|i| usable[i].clone())
                .ok_or_else(|| "no usable graphics adapter".to_owned())
        }));
    }
    c
}

/// Every adapter of the system (listed once, for the Graphics page).
pub(crate) fn adapters() -> &'static [String] {
    static LIST: OnceLock<Vec<String>> = OnceLock::new();
    LIST.get_or_init(|| {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
        let mut fut = std::pin::pin!(instance.enumerate_adapters(wgpu::Backends::all()));
        let mut cx = std::task::Context::from_waker(std::task::Waker::noop());
        // Native backends answer at once; anything else lists nothing.
        match fut.as_mut().poll(&mut cx) {
            std::task::Poll::Ready(list) => {
                let mut names: Vec<String> =
                    list.iter().map(|a| adapter_label(&a.get_info())).collect();
                names.dedup();
                names
            }
            std::task::Poll::Pending => Vec::new(),
        }
    })
}

/// Where the graphics log is written.
pub(crate) fn log_path() -> Option<PathBuf> {
    crate::settings::data_dir().map(|d| d.join("graphics.log"))
}

/// The graphics log's text.
pub(crate) fn log_text(
    info: &wgpu::AdapterInfo,
    wanted: Option<&str>,
    samples: u32,
    supported: &[u32],
) -> String {
    let mut s = String::from("meshtard graphics log\n\n");
    s += &format!("Version: {}\n", env!("CARGO_PKG_VERSION"));
    s += &format!("Adapter in use: {}\n", adapter_label(info));
    s += &format!(
        "Chosen in Preferences: {}\n",
        wanted.unwrap_or("(automatic)")
    );
    s += &format!("Device type: {:?}\n", info.device_type);
    s += &format!("Driver: {} {}\n", info.driver, info.driver_info);
    s += &format!(
        "Vendor / device id: {:#06x} / {:#06x}\n",
        info.vendor, info.device
    );
    s += &format!("Multisample anti-aliasing: {samples}x (supported: {supported:?})\n");
    s
}

/// Writes the graphics log (called at start-up).
pub(crate) fn write_log(text: &str) {
    let Some(path) = log_path() else { return };
    let _ = path.parent().map(std::fs::create_dir_all);
    if let Err(e) = std::fs::write(&path, text) {
        log::warn!("graphics log {}: {e}", path.display());
    }
}

/// Opens `file` in the image editor chosen on the Applications page, or with the
/// system's default application.
pub(crate) fn open_image_editor(editor: Option<&Path>, file: &Path) -> std::io::Result<()> {
    match editor {
        Some(app) if cfg!(target_os = "macos") && app.extension().is_some_and(|e| e == "app") => {
            std::process::Command::new("open")
                .arg("-a")
                .arg(app)
                .arg(file)
                .spawn()
                .map(|_| ())
        }
        Some(app) => std::process::Command::new(app)
            .arg(file)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .map(|_| ()),
        None => crate::presentation::open_with_system(&file.to_string_lossy()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn choose_takes_the_saved_gpu_or_the_best_one() {
        use wgpu::DeviceType as T;
        let list = vec![
            ("llvmpipe (Vulkan)".to_owned(), T::Cpu),
            ("Intel UHD (Vulkan)".to_owned(), T::IntegratedGpu),
            ("RTX 4090 (Vulkan)".to_owned(), T::DiscreteGpu),
        ];
        assert_eq!(choose(&list, Some("Intel UHD (Vulkan)")), Some(1));
        assert_eq!(choose(&list, None), Some(2));
        assert_eq!(
            choose(&list, Some("gone (Gl)")),
            Some(2),
            "missing: the best"
        );
        assert_eq!(choose(&list[..1], None), Some(0));
        assert_eq!(choose(&[], Some("x")), None);
    }

    #[test]
    fn log_names_the_adapter_and_samples() {
        let mut info = wgpu::AdapterInfo::new(wgpu::DeviceType::DiscreteGpu, wgpu::Backend::Vulkan);
        info.name = "Test GPU".into();
        info.vendor = 1;
        info.device = 2;
        info.driver = "drv".into();
        info.driver_info = "1.0".into();
        let text = log_text(&info, None, 4, &[1, 4]);
        assert!(text.contains("Test GPU (Vulkan)"), "{text}");
        assert!(text.contains("(automatic)"));
        assert!(text.contains("4x"));
    }
}
