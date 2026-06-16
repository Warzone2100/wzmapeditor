//! Editor icon asset, decoded for both the OS window and in-app egui display.

/// Embedded source PNG for the editor's icon (256x256 transparent render
/// of the Emplacement-MdART-pit).
pub const PNG_BYTES: &[u8] = include_bytes!("../icons/256x256.png");

/// Decode the icon for `eframe::NativeOptions`. Falls back to an empty
/// `IconData` if decode fails so a corrupt asset can't block startup.
#[cfg(not(target_arch = "wasm32"))]
pub fn for_window() -> egui::IconData {
    match image::load_from_memory(PNG_BYTES) {
        Ok(img) => {
            let rgba = img.to_rgba8();
            egui::IconData {
                width: rgba.width(),
                height: rgba.height(),
                rgba: rgba.into_raw(),
            }
        }
        Err(e) => {
            log::warn!("failed to decode window icon: {e}");
            egui::IconData::default()
        }
    }
}

/// Decode + resize the icon to `target_px` and upload it as an egui texture.
/// Resizing on the CPU keeps the GPU upload at display resolution rather
/// than the full 256, and lets the egui sampler use plain linear filtering.
/// Returns `None` if the embedded PNG is corrupt so a bad asset can't crash
/// startup or the settings window.
pub fn for_egui(ctx: &egui::Context, target_px: u32) -> Option<egui::TextureHandle> {
    let img = match image::load_from_memory(PNG_BYTES) {
        Ok(img) => img,
        Err(e) => {
            log::warn!("failed to decode editor icon for egui: {e}");
            return None;
        }
    };
    let resized = image::imageops::resize(
        &img.to_rgba8(),
        target_px,
        target_px,
        image::imageops::FilterType::Lanczos3,
    );
    let color = egui::ColorImage::from_rgba_unmultiplied(
        [target_px as usize, target_px as usize],
        resized.as_raw(),
    );
    Some(ctx.load_texture("editor_icon", color, egui::TextureOptions::LINEAR))
}
