//! wzmapeditor - Warzone 2100 map editor.

// mimalloc as the global allocator (M-MIMALLOC-APPS, up to ~25% faster on
// allocation-heavy hot paths). Native only; wasm uses its default allocator.
#[cfg(not(target_arch = "wasm32"))]
use mimalloc::MiMalloc;

#[cfg(not(target_arch = "wasm32"))]
#[global_allocator]
static GLOBAL: MiMalloc = MiMalloc;

mod app;
mod assets;
mod autosave;
mod balance;
mod config;
mod designer;
mod generator;
mod icon;
mod keybindings;
mod launch_sentinel;
#[cfg(not(target_arch = "wasm32"))]
mod logger;
mod map;
#[cfg(not(target_arch = "wasm32"))]
mod panic_logger;
mod publish;
mod startup;
mod thumbnails;
mod tools;
mod ui;
mod update_check;
mod viewport;
mod viewshed;
#[cfg(target_arch = "wasm32")]
mod web;
#[cfg(target_arch = "wasm32")]
mod web_data;
#[cfg(target_arch = "wasm32")]
mod web_map_io;

#[cfg(not(target_arch = "wasm32"))]
fn main() -> eframe::Result {
    // Create the Output-panel channel before installing the logger so
    // editor-crate warnings/errors emitted during startup are captured and
    // replayed once `EditorApp` takes ownership of the receiver.
    let (output_log, panel_tx) = app::output_log::OutputLog::new();

    let log_file = config::log_file_path();
    if let Err(e) = logger::init(&log_file, panel_tx) {
        eprintln!("failed to install logger: {e}");
    }
    panic_logger::install();
    log::info!("Starting wzmapeditor");
    log::info!("Log file: {}", log_file.display());

    // basis-universal transcoder for KTX2/UASTC texture decoding.
    basis_universal::transcoding::transcoder_init();

    // Load the persisted config once at startup to pick the wgpu backend.
    // `EditorApp::new` reads it again; the second read is cheap and keeps
    // the app init path unchanged.
    let mut startup_config = config::EditorConfig::load();

    if let Some(prev) = launch_sentinel::consume() {
        log::warn!(
            "The previous launch crashed during startup using the {} graphics backend. If this keeps happening, edit graphics_backend in {}/wzmapeditor.json or delete it to retry.",
            prev.label(),
            config::config_dir().display()
        );
        if prev == startup_config.graphics_backend {
            let alt = next_backend_after_crash(prev);
            if alt != prev {
                log::warn!(
                    "Switching to {} for this launch and saving the choice. Change it in Settings > Rendering > Graphics Backend if you'd rather pick yourself.",
                    alt.label()
                );
                startup_config.graphics_backend = alt;
                startup_config.save();
            }
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
            .with_title(concat!("wzmapeditor - ", env!("CARGO_PKG_VERSION")))
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
        concat!("wzmapeditor - ", env!("CARGO_PKG_VERSION")),
        native_options,
        Box::new(move |cc| {
            let log = output_log_slot
                .take()
                .expect("EditorApp factory invoked more than once");
            Ok(Box::new(app::EditorApp::new(cc, log)))
        }),
    )
}

#[cfg(not(target_arch = "wasm32"))]
fn backend_flags_for(cfg: &config::EditorConfig) -> wgpu::Backends {
    match cfg.graphics_backend {
        config::GraphicsBackend::Dx12 => wgpu::Backends::DX12,
        config::GraphicsBackend::Vulkan => wgpu::Backends::VULKAN,
        config::GraphicsBackend::Metal => wgpu::Backends::METAL,
        config::GraphicsBackend::OpenGl => wgpu::Backends::GL,
    }
}

/// Walk one step around the platform's backend preference list,
/// snapping to the default if `prev` isn't in the list (cross-OS config copy).
#[cfg(not(target_arch = "wasm32"))]
fn next_backend_after_crash(prev: config::GraphicsBackend) -> config::GraphicsBackend {
    let chain = config::GraphicsBackend::available_for_platform();
    match chain.iter().position(|b| *b == prev) {
        Some(pos) => chain[(pos + 1) % chain.len()],
        None => chain[0],
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
#[cfg(not(target_arch = "wasm32"))]
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

// Web entry point. Trunk builds this binary to wasm and wasm-bindgen treats
// `main` as the start function; the egui scene is hosted in a `<canvas>`.
#[cfg(target_arch = "wasm32")]
fn main() {
    use eframe::wasm_bindgen::JsCast as _;

    console_error_panic_hook::set_once();
    eframe::WebLogger::init(log::LevelFilter::Info).ok();
    log::info!("Starting wzmapeditor (web)");

    // The 3D viewport pipelines declare a Depth32Float attachment, so the web
    // painter's egui pass needs a matching depth buffer (native sets the same
    // via NativeOptions::depth_buffer).
    let web_options = eframe::WebOptions {
        depth_buffer: 32,
        ..eframe::WebOptions::default()
    };

    wasm_bindgen_futures::spawn_local(async {
        let document = web_sys::window()
            .expect("no window")
            .document()
            .expect("no document");

        document.set_title(concat!("wzmapeditor - ", env!("CARGO_PKG_VERSION")));

        let canvas = document
            .get_element_by_id("the_canvas_id")
            .expect("missing #the_canvas_id element")
            .dyn_into::<web_sys::HtmlCanvasElement>()
            .expect("#the_canvas_id was not a HtmlCanvasElement");

        let start_result = eframe::WebRunner::new()
            .start(
                canvas,
                web_options,
                Box::new(|cc| {
                    let (output_log, _panel_tx) = app::output_log::OutputLog::new();
                    Ok(Box::new(app::EditorApp::new(cc, output_log)))
                }),
            )
            .await;

        if let Some(loading_text) = document.get_element_by_id("loading_text") {
            match start_result {
                Ok(()) => loading_text.remove(),
                Err(e) => {
                    loading_text.set_inner_html(
                        "<p>The app has crashed. See the developer console for details.</p>",
                    );
                    panic!("failed to start eframe: {e:?}");
                }
            }
        }
    });
}
