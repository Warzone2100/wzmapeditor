//! wzmapeditor - Warzone 2100 map editor.

use mimalloc::MiMalloc;

// mimalloc as the global allocator (M-MIMALLOC-APPS, up to ~25% faster on
// allocation-heavy hot paths).
#[global_allocator]
static GLOBAL: MiMalloc = MiMalloc;

mod app;
mod autosave;
mod balance;
mod config;
mod designer;
mod generator;
mod icon;
mod keybindings;
mod launch_sentinel;
mod logger;
mod map;
mod startup;
mod thumbnails;
mod tools;
mod ui;
mod viewport;
mod viewshed;

fn main() -> eframe::Result {
    // Create the Output-panel channel before installing the logger so
    // editor-crate warnings/errors emitted during startup are captured and
    // replayed once `EditorApp` takes ownership of the receiver.
    let (output_log, panel_tx) = app::output_log::OutputLog::new();

    let log_file = config::log_file_path();
    if let Err(e) = logger::init(&log_file, panel_tx) {
        eprintln!("failed to install logger: {e}");
    }
    log::info!("Starting wzmapeditor");
    log::info!("Log file: {}", log_file.display());

    // basis-universal transcoder for KTX2/UASTC texture decoding.
    basis_universal::transcoding::transcoder_init();

    // Load the persisted config once at startup to pick the wgpu backend.
    // `EditorApp::new` reads it again; the second read is cheap and keeps
    // the app init path unchanged.
    #[cfg_attr(
        not(target_os = "windows"),
        expect(
            unused_mut,
            reason = "launch-sentinel fallback only mutates on Windows"
        )
    )]
    let mut startup_config = config::EditorConfig::load();

    if let Some(prev) = launch_sentinel::consume() {
        log::warn!(
            "The previous launch crashed during startup using the {} graphics backend. If this keeps happening, edit graphics_backend in {}/wzmapeditor.json or delete it to retry.",
            prev.label(),
            config::config_dir().display()
        );
        #[cfg(target_os = "windows")]
        if prev == startup_config.graphics_backend {
            let alt = match prev {
                config::GraphicsBackend::Dx12 => config::GraphicsBackend::Vulkan,
                config::GraphicsBackend::Vulkan | config::GraphicsBackend::OpenGl => {
                    config::GraphicsBackend::Dx12
                }
            };
            log::warn!(
                "Switching to {} for this launch and saving the choice. Change it in Settings > Rendering > Graphics Backend if you'd rather pick yourself.",
                alt.label()
            );
            startup_config.graphics_backend = alt;
            startup_config.save();
        }
    }

    log::info!(
        "Graphics backend preference: {}",
        startup_config.graphics_backend.label()
    );
    log::info!(
        "Present mode preference: {}",
        startup_config.present_mode.label()
    );

    let native_options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1400.0, 900.0])
            .with_min_inner_size([800.0, 600.0])
            .with_title("wzmapeditor")
            .with_icon(icon::for_window()),
        renderer: eframe::Renderer::Wgpu,
        // 3D terrain pipeline requires a depth attachment.
        depth_buffer: 32,
        wgpu_options: eframe::egui_wgpu::WgpuConfiguration {
            present_mode: present_mode_for(&startup_config),
            // Cap CPU-side frames-in-flight at 1 to minimize input latency.
            // Vulkan on Windows otherwise queues ~2 frames, feeling laggier
            // than DX12's flip-model swapchain (DXGI pins latency to 1 via
            // waitable objects automatically). Setting this to 1 closes the
            // gap on Vulkan and shaves a frame off DX12 in practice.
            desired_maximum_frame_latency: Some(1),
            wgpu_setup: eframe::egui_wgpu::WgpuSetup::CreateNew(
                eframe::egui_wgpu::WgpuSetupCreateNew {
                    instance_descriptor: wgpu::InstanceDescriptor {
                        backends: backend_flags_for(&startup_config),
                        // Debug builds default to VALIDATION + DEBUG +
                        // VALIDATION_INDIRECT_CALL, which loads the Vulkan
                        // validation layers and costs ~5-15ms/frame on
                        // draw-heavy scenes. Start empty and let dev opt
                        // back in with WGPU_VALIDATION=1 or WGPU_DEBUG=1.
                        flags: wgpu::InstanceFlags::empty().with_env(),
                        ..wgpu::InstanceDescriptor::new_without_display_handle()
                    },
                    // Discrete GPU over integrated graphics.
                    power_preference: wgpu::PowerPreference::HighPerformance,
                    ..eframe::egui_wgpu::WgpuSetupCreateNew::without_display_handle()
                },
            ),
            ..Default::default()
        },
        ..Default::default()
    };

    launch_sentinel::arm(startup_config.graphics_backend);

    let mut output_log_slot = Some(output_log);
    eframe::run_native(
        "wzmapeditor",
        native_options,
        Box::new(move |cc| {
            let log = output_log_slot
                .take()
                .expect("EditorApp factory invoked more than once");
            Ok(Box::new(app::EditorApp::new(cc, log)))
        }),
    )
}

/// On Windows the user picks Direct3D 12 (default) or Vulkan. Other
/// platforms ignore the choice and use the OS primary backend (Metal on
/// macOS, Vulkan on Linux).
fn backend_flags_for(cfg: &config::EditorConfig) -> wgpu::Backends {
    #[cfg(target_os = "windows")]
    {
        match cfg.graphics_backend {
            config::GraphicsBackend::Dx12 => eframe::wgpu::Backends::DX12,
            config::GraphicsBackend::Vulkan => eframe::wgpu::Backends::VULKAN,
            config::GraphicsBackend::OpenGl => eframe::wgpu::Backends::GL,
        }
    }
    #[cfg(not(target_os = "windows"))]
    {
        let _ = cfg;
        wgpu::Backends::PRIMARY
    }
}

/// `SmartVsync` resolves to `AutoVsync` on every platform. `AutoVsync`
/// picks `FifoRelaxed` first, then `Fifo`. Both block at vblank, so the
/// frame rate is capped to the monitor's refresh rate. The older policy
/// of `Mailbox` on Vulkan still vsynced presentation but kept the producer
/// running unbounded (Mailbox replaces queued frames rather than blocking),
/// so the GPU rendered hundreds of FPS the display never showed.
/// `desired_maximum_frame_latency: Some(1)` at the eframe level keeps the
/// FIFO queue tight enough that input latency matches Mailbox without the
/// runaway frame rate.
fn present_mode_for(cfg: &config::EditorConfig) -> wgpu::PresentMode {
    match cfg.present_mode {
        config::PresentMode::SmartVsync | config::PresentMode::AutoVsync => {
            wgpu::PresentMode::AutoVsync
        }
        config::PresentMode::AutoNoVsync => wgpu::PresentMode::AutoNoVsync,
        config::PresentMode::Fifo => wgpu::PresentMode::Fifo,
        config::PresentMode::Mailbox => wgpu::PresentMode::Mailbox,
        config::PresentMode::Immediate => wgpu::PresentMode::Immediate,
    }
}
