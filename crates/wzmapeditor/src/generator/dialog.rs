//! Generator dialog UI: egui window with configuration controls and progress.

use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};

use crate::app::EditorApp;
use crate::config::Tileset;
use crate::tools::MirrorMode;

use super::{GeneratorConfig, GeneratorResult, ProgressReporter};

#[derive(Default)]
pub struct GeneratorDialog {
    pub open: bool,
    pub config: GeneratorConfig,
    pub gen_rx: Option<std::sync::mpsc::Receiver<GeneratorResult>>,
    /// Progress in thousandths (0-1000), shared with the background thread.
    pub progress: Option<Arc<AtomicU32>>,
    pub step_label: Option<Arc<Mutex<String>>>,
    pub error: Option<String>,
    /// Seed actually used; shown after generation so the run is reproducible.
    pub used_seed: Option<u64>,
}

#[expect(
    clippy::missing_fields_in_debug,
    reason = "gen_rx/progress/step_label are runtime-only channels; showing them adds noise"
)]
impl std::fmt::Debug for GeneratorDialog {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GeneratorDialog")
            .field("open", &self.open)
            .field("generating", &self.gen_rx.is_some())
            .finish()
    }
}

pub(crate) fn show_generator_dialog(ctx: &egui::Context, app: &mut EditorApp) {
    poll_generation_result(app, ctx);

    let is_generating = app.generator_dialog.gen_rx.is_some();

    let mut open = app.generator_dialog.open;
    egui::Window::new("Generate Map")
        .open(&mut open)
        .collapsible(false)
        .resizable(false)
        .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
        .default_width(380.0)
        .show(ctx, |ui| {
            if is_generating {
                show_progress(ui, app);
            } else {
                show_config(ui, app);
            }
        });

    if app.generator_dialog.open {
        app.generator_dialog.open = open;
    }
}

fn show_config(ui: &mut egui::Ui, app: &mut EditorApp) {
    let config = &mut app.generator_dialog.config;

    egui::Frame::new()
        .fill(egui::Color32::from_rgb(80, 50, 20))
        .inner_margin(egui::Margin::symmetric(8, 6))
        .corner_radius(4)
        .show(ui, |ui| {
            ui.colored_label(egui::Color32::from_rgb(255, 200, 120), "⚠ EXPERIMENTAL");
        });
    ui.add_space(8.0);

    // Resolve the "0 = random" sentinel up front so the user can see and copy
    // the seed before clicking Generate.
    if config.seed == 0 {
        config.seed = fastrand::u64(..);
    }

    if let Some(ref err) = app.generator_dialog.error {
        ui.colored_label(egui::Color32::RED, err);
        ui.add_space(4.0);
    }

    ui.heading("Layout");
    ui.add_space(4.0);

    ui.horizontal(|ui| {
        ui.label("Name:");
        ui.text_edit_singleline(&mut config.map_name);
    });

    let sizes: &[u32] = &[48, 64, 96, 128, 192, 250];
    ui.horizontal(|ui| {
        ui.label("Width:");
        egui::ComboBox::from_id_salt("gen_width")
            .selected_text(format!("{}", config.width))
            .show_ui(ui, |ui| {
                for &s in sizes {
                    ui.selectable_value(&mut config.width, s, format!("{s}"));
                }
            });
        ui.label("Height:");
        egui::ComboBox::from_id_salt("gen_height")
            .selected_text(format!("{}", config.height))
            .show_ui(ui, |ui| {
                for &s in sizes {
                    ui.selectable_value(&mut config.height, s, format!("{s}"));
                }
            });
    });

    ui.horizontal(|ui| {
        ui.label("Tileset:");
        egui::ComboBox::from_id_salt("gen_tileset")
            .selected_text(config.tileset.to_string())
            .show_ui(ui, |ui| {
                for ts in Tileset::ALL {
                    ui.selectable_value(&mut config.tileset, ts, ts.to_string());
                }
            });
    });

    ui.horizontal(|ui| {
        ui.label("Players:");
        egui::ComboBox::from_id_salt("gen_players")
            .selected_text(format!("{}", config.players))
            .show_ui(ui, |ui| {
                for &p in &[2u8, 3, 4, 5, 6, 7, 8, 10] {
                    ui.selectable_value(&mut config.players, p, format!("{p}"));
                }
            });
    });

    ui.horizontal(|ui| {
        ui.label("Symmetry:");
        let modes = [
            (MirrorMode::None, "None"),
            (MirrorMode::Vertical, "Vertical (left/right)"),
            (MirrorMode::Horizontal, "Horizontal (top/bottom)"),
            (MirrorMode::Both, "Both (4-way)"),
            (MirrorMode::Diagonal, "Diagonal (rotational)"),
        ];
        egui::ComboBox::from_id_salt("gen_symmetry")
            .selected_text(
                modes
                    .iter()
                    .find(|(m, _)| *m == config.symmetry)
                    .map_or("None", |(_, label)| label),
            )
            .show_ui(ui, |ui| {
                for (mode, label) in modes {
                    let enabled = mode != MirrorMode::Diagonal || config.width == config.height;
                    ui.add_enabled_ui(enabled, |ui| {
                        ui.selectable_value(&mut config.symmetry, mode, label);
                    });
                }
            });
    });

    // Diagonal symmetry requires a square map; fall back if dims diverged.
    if config.symmetry == MirrorMode::Diagonal && config.width != config.height {
        config.symmetry = MirrorMode::Both;
    }

    ui.add_space(8.0);

    ui.heading("Terrain");
    ui.add_space(4.0);

    ui.horizontal(|ui| {
        ui.label("Height levels:");
        ui.add(egui::Slider::new(&mut config.height_levels, 3..=5));
    });

    ui.horizontal(|ui| {
        ui.label("Level frequency:");
        ui.add(
            egui::Slider::new(&mut config.level_frequency, 0.0..=1.0)
                .custom_formatter(|v, _| format!("{:.0}%", v * 100.0)),
        );
    });

    ui.horizontal(|ui| {
        ui.label("Height variation:");
        ui.add(
            egui::Slider::new(&mut config.height_variation, 0.0..=1.0)
                .custom_formatter(|v, _| format!("{:.0}%", v * 100.0)),
        );
    });

    ui.horizontal(|ui| {
        ui.label("Flatness:");
        ui.add(
            egui::Slider::new(&mut config.flatness, 0.0..=1.0)
                .custom_formatter(|v, _| format!("{:.0}%", v * 100.0)),
        );
    });

    ui.horizontal(|ui| {
        ui.label("Water bodies:");
        ui.add(egui::Slider::new(&mut config.water_spawns, 0..=5));
    });

    ui.add_space(8.0);

    ui.heading("Resources");
    ui.add_space(4.0);

    ui.horizontal(|ui| {
        ui.label("Oil per base:");
        ui.add(egui::Slider::new(&mut config.base_oil, 0..=16));
    });

    ui.horizontal(|ui| {
        ui.label("Extra oil:");
        ui.add(egui::Slider::new(&mut config.extra_oil, 0..=99));
    });

    ui.horizontal(|ui| {
        ui.label("Trucks per player:");
        ui.add(egui::Slider::new(&mut config.trucks_per_player, 0..=15));
    });

    ui.horizontal(|ui| {
        ui.label("Oil drums (pickups):");
        ui.add(egui::Slider::new(&mut config.oil_drums, 0..=30));
    });

    ui.checkbox(&mut config.scatter_features, "Scatter decorative features");
    if config.scatter_features {
        ui.horizontal(|ui| {
            ui.label("Feature density:");
            ui.add(
                egui::Slider::new(&mut config.feature_density, 0.0..=1.0)
                    .custom_formatter(|v, _| format!("{:.0}%", v * 100.0)),
            );
        });
    }

    ui.checkbox(&mut config.scavengers, "Place scavenger bases");
    if config.scavengers {
        ui.horizontal(|ui| {
            ui.label("Scavenger bases:");
            ui.add(egui::Slider::new(&mut config.scavenger_bases, 0..=8));
        });
    }

    ui.add_space(8.0);

    ui.heading("Advanced");
    ui.add_space(4.0);

    ui.horizontal(|ui| {
        ui.label("Seed:");
        let mut seed_str = config.seed.to_string();
        let response = ui.text_edit_singleline(&mut seed_str);
        if response.changed() {
            // Keep the prior seed on parse failure so the field doesn't snap to
            // 0 while the user is mid-edit.
            if let Ok(parsed) = seed_str.parse::<u64>() {
                config.seed = parsed;
            }
        }
        if ui.button("Randomize").clicked() {
            config.seed = fastrand::u64(..);
        }
    });

    if let Some(seed) = app.generator_dialog.used_seed {
        ui.label(format!("Last seed: {seed}"));
    }

    ui.add_space(12.0);

    if ui
        .add_sized([ui.available_width(), 32.0], egui::Button::new("Generate"))
        .clicked()
    {
        start_generation(app, ui.ctx());
    }
}

fn show_progress(ui: &mut egui::Ui, app: &mut EditorApp) {
    ui.add_space(8.0);

    let progress_frac = app
        .generator_dialog
        .progress
        .as_ref()
        .map_or(0.0, |p| p.load(Ordering::Relaxed) as f32 / 1000.0);

    let step = app
        .generator_dialog
        .step_label
        .as_ref()
        .and_then(|l| l.lock().ok().map(|s| s.clone()))
        .unwrap_or_default();

    ui.label(format!("Generating... {step}"));
    ui.add(egui::ProgressBar::new(progress_frac).show_percentage());
    ui.add_space(8.0);

    ui.ctx().request_repaint();
}

fn start_generation(app: &mut EditorApp, ctx: &egui::Context) {
    let mut config = app.generator_dialog.config.clone();

    if config.seed == 0 {
        config.seed = fastrand::u64(..);
    }
    app.generator_dialog.used_seed = Some(config.seed);
    app.generator_dialog.error = None;

    let progress = Arc::new(AtomicU32::new(0));
    let label = Arc::new(Mutex::new(String::new()));
    let (tx, rx) = std::sync::mpsc::channel();
    let progress_clone = progress.clone();
    let label_clone = label.clone();
    let ctx_clone = ctx.clone();

    let work = move || {
        let reporter = ProgressReporter::new(progress_clone, label_clone);
        let result = super::pipeline::generate_map(&config, &reporter);
        let _ = tx.send(result);
        ctx_clone.request_repaint();
    };
    // No usable OS threads in the browser; generation is pure CPU, run inline.
    #[cfg(not(target_arch = "wasm32"))]
    std::thread::spawn(work);
    #[cfg(target_arch = "wasm32")]
    work();

    app.generator_dialog.gen_rx = Some(rx);
    app.generator_dialog.progress = Some(progress);
    app.generator_dialog.step_label = Some(label);
}

fn poll_generation_result(app: &mut EditorApp, _ctx: &egui::Context) {
    let result = app
        .generator_dialog
        .gen_rx
        .as_ref()
        .and_then(|rx| rx.try_recv().ok());

    if let Some(result) = result {
        app.generator_dialog.gen_rx = None;
        app.generator_dialog.progress = None;
        app.generator_dialog.step_label = None;

        match result {
            Ok(map) => {
                app.load_map(map, None, None, None);
                app.generator_dialog.open = false;
                app.log("Map generated successfully".to_string());
            }
            Err(e) => {
                app.generator_dialog.error = Some(format!("Generation failed: {e}"));
                app.log(format!("Map generation failed: {e}"));
            }
        }
    }
}
