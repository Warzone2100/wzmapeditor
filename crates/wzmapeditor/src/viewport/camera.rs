//! Free-look camera with WASD movement and mouse control.

use glam::{Mat4, Vec3};

/// Default mouse look sensitivity (radians per pixel of drag).
const DEFAULT_LOOK_SENSITIVITY: f32 = 0.003;

/// Free-look camera for the 3D viewport.
#[derive(Clone, Debug)]
pub struct Camera {
    /// Camera world position.
    pub position: Vec3,
    /// Yaw angle in radians (rotation around Y axis).
    pub yaw: f32,
    /// Pitch angle in radians (rotation around X axis, clamped).
    pub pitch: f32,
    /// Vertical field of view in radians.
    pub fov: f32,
    /// Aspect ratio (width / height).
    pub aspect: f32,
    /// Near clipping plane.
    pub near: f32,
    /// Far clipping plane.
    pub far: f32,
    /// Movement speed (world units per second).
    pub move_speed: f32,
    /// Mouse look sensitivity.
    pub look_sensitivity: f32,
}

impl Camera {
    /// Create a camera positioned to look down at the map center.
    ///
    /// `map_width` and `map_height` are in tiles; tile spacing is 128 world units.
    pub fn for_map(map_width: u32, map_height: u32) -> Self {
        let tile_size = 128.0_f32;
        let center_x = map_width as f32 * tile_size * 0.5;
        let center_z = map_height as f32 * tile_size * 0.5;
        let map_extent = (map_width.max(map_height) as f32) * tile_size;

        // Position above and behind center, looking down at ~45 degrees
        let height = map_extent * 0.6;
        let offset_back = map_extent * 0.4;

        log::info!(
            "Camera::for_map({map_width}x{map_height}) center=({center_x}, {center_z}), height={height}, offset_back={offset_back}, extent={map_extent}"
        );

        Self {
            // Place camera behind the map (in -Z direction), looking toward center
            position: Vec3::new(center_x, height, center_z + offset_back),
            // yaw = PI means forward is along -Z (toward map center)
            yaw: std::f32::consts::PI,
            pitch: -0.75,                     // ~43 degrees down
            fov: std::f32::consts::FRAC_PI_4, // 45 degrees
            aspect: 16.0 / 9.0,
            near: 10.0,
            far: map_extent * 4.0,
            move_speed: map_extent * 0.25,
            look_sensitivity: DEFAULT_LOOK_SENSITIVITY,
        }
    }

    /// Move the camera to look directly at the given world position.
    ///
    /// `target_y` is the terrain/object height at the target so the camera
    /// aims at the correct elevation rather than ground level.
    pub fn focus_on(&mut self, world_x: f32, world_z: f32, target_y: f32) {
        let distance = 1000.0;
        let pitch = -0.93_f32; // ~53 degrees down
        let yaw = std::f32::consts::PI; // face toward -Z

        // Reverse the forward vector to get camera offset from target.
        let (sin_yaw, cos_yaw) = yaw.sin_cos();
        let (sin_pitch, cos_pitch) = pitch.sin_cos();
        let forward = Vec3::new(cos_pitch * sin_yaw, sin_pitch, cos_pitch * cos_yaw);

        self.yaw = yaw;
        self.pitch = pitch;
        self.position = Vec3::new(world_x, target_y, world_z) - forward * distance;
    }

    /// Compute the view matrix (world -> camera space).
    pub fn view_matrix(&self) -> Mat4 {
        let (sin_yaw, cos_yaw) = self.yaw.sin_cos();
        let (sin_pitch, cos_pitch) = self.pitch.sin_cos();

        let forward = Vec3::new(cos_pitch * sin_yaw, sin_pitch, cos_pitch * cos_yaw);
        let target = self.position + forward;
        Mat4::look_at_rh(self.position, target, Vec3::Y)
    }

    /// Compute the perspective projection matrix.
    pub fn projection_matrix(&self) -> Mat4 {
        Mat4::perspective_rh(self.fov, self.aspect, self.near, self.far)
    }

    /// Compute the combined view-projection matrix.
    pub fn view_projection_matrix(&self) -> Mat4 {
        self.projection_matrix() * self.view_matrix()
    }

    /// Camera right and up vectors for billboard orientation.
    pub fn billboard_axes(&self) -> (Vec3, Vec3) {
        let (sin_yaw, cos_yaw) = self.yaw.sin_cos();
        let (sin_pitch, cos_pitch) = self.pitch.sin_cos();
        let forward = Vec3::new(cos_pitch * sin_yaw, sin_pitch, cos_pitch * cos_yaw);
        let right = forward.cross(Vec3::Y).normalize_or_zero();
        let up = right.cross(forward).normalize_or_zero();
        (right, up)
    }

    /// Forward direction on the XZ plane (for movement).
    fn forward_xz(&self) -> Vec3 {
        let (sin_yaw, cos_yaw) = self.yaw.sin_cos();
        Vec3::new(sin_yaw, 0.0, cos_yaw).normalize_or_zero()
    }

    /// Right direction on the XZ plane (for movement).
    fn right_xz(&self) -> Vec3 {
        self.forward_xz().cross(Vec3::Y).normalize_or_zero()
    }

    /// Process input - Unity scene view style:
    /// - Hold RMB + WASD/QE to fly
    /// - Hold RMB + drag to look around
    /// - Middle-click drag to pan
    /// - Scroll to zoom (move forward/back along look direction)
    /// - Shift for faster movement
    pub fn process_input(&mut self, response: &egui::Response, ctx: &egui::Context, dt: f32) {
        let rmb_held = response.dragged_by(egui::PointerButton::Secondary)
            || ctx.input(|i| i.pointer.button_down(egui::PointerButton::Secondary));

        if response.dragged_by(egui::PointerButton::Secondary) {
            let delta = response.drag_delta();
            self.yaw -= delta.x * self.look_sensitivity;
            self.pitch -= delta.y * self.look_sensitivity;
            self.pitch = self.pitch.clamp(
                -std::f32::consts::FRAC_PI_2 + 0.01,
                std::f32::consts::FRAC_PI_2 - 0.01,
            );
        }

        if response.dragged_by(egui::PointerButton::Middle) {
            let delta = response.drag_delta();
            let right = self.right_xz();
            let pan_speed = self.move_speed * 0.001;
            self.position -= right * delta.x * pan_speed;
            self.position.y += delta.y * pan_speed;
        }

        if response.hovered() {
            let smooth_scroll = ctx.input(|i| i.smooth_scroll_delta.y);
            let raw_scroll = ctx.input(|i| {
                i.raw
                    .events
                    .iter()
                    .filter_map(|e| match e {
                        egui::Event::MouseWheel { delta, .. } => Some(delta.y),
                        _ => None,
                    })
                    .sum::<f32>()
            });
            let shift_held = ctx.input(|i| i.modifiers.shift);
            if rmb_held || shift_held {
                if raw_scroll != 0.0 {
                    // Exponential per-unit scaling. ln(1.2)/50 gives ~1.2x
                    // per canonical wheel notch (~50 units), and because
                    // exp(a)*exp(b) = exp(a+b), spreading the same total
                    // scroll across more frames at high fps multiplies to
                    // the same final factor. A bare sign-based factor
                    // compounded once per frame, which ran away at 400 fps.
                    let factor = (raw_scroll * (1.2_f32.ln() / 50.0)).exp();
                    self.move_speed = (self.move_speed * factor).clamp(200.0, 50_000.0);
                }
            } else if smooth_scroll != 0.0 {
                let zoom_step = ((self.position.y - 10.0).max(50.0) * 0.10).clamp(20.0, 5_000.0);
                let look_dir = self.forward_3d();
                self.position += look_dir * smooth_scroll * zoom_step;
                self.position.y = self.position.y.max(10.0);
            }
        }

        if rmb_held {
            let speed_multiplier = if ctx.input(|i| i.modifiers.shift) {
                3.0
            } else {
                1.0
            };
            let move_amount = self.move_speed * dt * speed_multiplier;

            let forward = self.forward_3d();
            let right = self.right_xz();

            ctx.input(|i| {
                if i.key_down(egui::Key::W) || i.key_down(egui::Key::ArrowUp) {
                    self.position += forward * move_amount;
                }
                if i.key_down(egui::Key::S) || i.key_down(egui::Key::ArrowDown) {
                    self.position -= forward * move_amount;
                }
                if i.key_down(egui::Key::A) || i.key_down(egui::Key::ArrowLeft) {
                    self.position -= right * move_amount;
                }
                if i.key_down(egui::Key::D) || i.key_down(egui::Key::ArrowRight) {
                    self.position += right * move_amount;
                }
                if i.key_down(egui::Key::Q) {
                    self.position.y -= move_amount;
                }
                if i.key_down(egui::Key::E) {
                    self.position.y += move_amount;
                }
            });

            ctx.request_repaint();
        }
    }

    /// Full 3D forward direction (includes pitch).
    fn forward_3d(&self) -> Vec3 {
        let (sin_yaw, cos_yaw) = self.yaw.sin_cos();
        let (sin_pitch, cos_pitch) = self.pitch.sin_cos();
        Vec3::new(cos_pitch * sin_yaw, sin_pitch, cos_pitch * cos_yaw)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn for_map_positions_camera_above_center() {
        let cam = Camera::for_map(10, 10);
        let tile_size = 128.0_f32;
        let center = 10.0 * tile_size * 0.5;
        // X should be at map center
        assert!((cam.position.x - center).abs() < 1.0);
        // Y should be well above 0
        assert!(cam.position.y > 100.0);
        // Z should be behind center (greater, since yaw=PI faces -Z)
        assert!(cam.position.z > center);
    }

    #[test]
    fn for_map_looks_down() {
        let cam = Camera::for_map(20, 20);
        // Pitch should be negative (looking down)
        assert!(cam.pitch < 0.0);
        // Yaw should be PI (facing toward -Z = toward map center)
        assert!((cam.yaw - std::f32::consts::PI).abs() < 0.01);
    }

    #[test]
    fn focus_on_centers_target() {
        let mut cam = Camera::for_map(10, 10);
        cam.focus_on(1000.0, 2000.0, 0.0);
        // Camera should be above the target
        assert!(cam.position.y > 0.0);
        // The forward ray from the camera should pass through (1000, 0, 2000).
        let (sin_yaw, cos_yaw) = cam.yaw.sin_cos();
        let (sin_pitch, cos_pitch) = cam.pitch.sin_cos();
        let forward = Vec3::new(cos_pitch * sin_yaw, sin_pitch, cos_pitch * cos_yaw);
        let t = -cam.position.y / forward.y;
        let hit_x = cam.position.x + forward.x * t;
        let hit_z = cam.position.z + forward.z * t;
        assert!((hit_x - 1000.0).abs() < 1.0, "hit_x={hit_x}");
        assert!((hit_z - 2000.0).abs() < 1.0, "hit_z={hit_z}");
    }

    #[test]
    fn focus_on_looks_down_at_target() {
        let mut cam = Camera::for_map(10, 10);
        cam.focus_on(500.0, 500.0, 0.0);
        assert!(cam.pitch < 0.0);
        assert!((cam.yaw - std::f32::consts::PI).abs() < 0.01);
    }

    #[test]
    fn focus_on_different_positions() {
        let mut cam = Camera::for_map(10, 10);
        cam.focus_on(0.0, 0.0, 0.0);
        assert!(cam.position.x.abs() < 1.0);

        cam.focus_on(5000.0, 3000.0, 0.0);
        assert!((cam.position.x - 5000.0).abs() < 1.0);
    }

    #[test]
    fn view_matrix_is_valid() {
        let cam = Camera::for_map(10, 10);
        let view = cam.view_matrix();
        // View matrix determinant should be non-zero (invertible)
        assert!(view.determinant().abs() > 1e-6);
    }

    #[test]
    fn projection_matrix_is_valid() {
        let cam = Camera::for_map(10, 10);
        let proj = cam.projection_matrix();
        assert!(proj.determinant().abs() > 1e-6);
    }
}
