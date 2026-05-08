//! Weather particles replicating WZ2100's `atmos.cpp`. Particles spawn at
//! a fixed height above terrain around the camera, fall with gravity and
//! drift, and die on ground contact. Rain hitting water spawns short-lived
//! splash effects.

use glam::Vec3;
use wz_maplib::MapData;
use wz_maplib::constants::TILE_UNITS_F32 as TILE_UNITS;
use wz_maplib::io_wz::Weather;
use wz_maplib::terrain_types::TerrainTypeData;

use super::picking::sample_terrain_height_pub;
use super::water::is_water_tile;

// -- Particle pool --

/// WZ2100 caps to `MAP_MAXWIDTH * MAP_MAXHEIGHT` (~4096 for 64x64). The
/// editor's wider field needs comparable headroom.
const MAX_PARTICLES: usize = 4000;

/// Fixed absolute Y (not terrain-relative). Raised above WZ2100's 1000
/// so the editor's higher camera still sees rain.
const SPAWN_Y: f32 = 2000.0;

/// Horizontal spawn radius around camera XZ (world units), ~32 tiles.
const SPAWN_RADIUS: f32 = 4096.0;

/// WZ2100 spawns ~40 rain/s in a small viewport; scaled up for the
/// editor's wider area.
const RAIN_SPAWN_RATE: f32 = 300.0;
/// WZ2100 spawns ~20 snow/s; scaled up for the editor's wider area.
const SNOW_SPAWN_RATE: f32 = 100.0;

/// Rain fall speed (units/second). WZ2100 is 700-1000; scaled up ~4x
/// because the editor camera is ~4x further from the ground, where 700
/// looks like slow drizzle.
const RAIN_FALL_MIN: f32 = 3000.0;
const RAIN_FALL_MAX: f32 = 4500.0;
/// Rain horizontal drift. WZ2100: 0-50, scaled up.
const RAIN_DRIFT: f32 = 200.0;

/// Snow fall speed range. WZ2100: 80-120, scaled up.
const SNOW_FALL_MIN: f32 = 350.0;
const SNOW_FALL_MAX: f32 = 500.0;
/// Snow horizontal drift. WZ2100: +-40, scaled up.
const SNOW_DRIFT: f32 = 160.0;

/// Rain billboard half-dims. WZ2100 base 12x13 scaled ~4x for the
/// editor's longer viewing distance.
const RAIN_HALF_W: f32 = 3.0;
const RAIN_HALF_H: f32 = 40.0;

/// Snow billboard half-dims. WZ2100 base 1.6x1.6 scaled ~6x for the
/// editor's longer viewing distance.
const SNOW_HALF_W: f32 = 10.0;
const SNOW_HALF_H: f32 = 10.0;

/// Only sample terrain when particle Y is below this. WZ2100's
/// `TILE_MAX_HEIGHT` = 255 * `ELEVATION_SCALE(2)` = 510.
const TILE_MAX_HEIGHT: f32 = 510.0;

/// Cull particles beyond this distance from the camera (squared).
const CULL_RADIUS_SQ: f32 = 5000.0 * 5000.0;

// -- Splash effects --

const MAX_SPLASHES: usize = 128;

/// Splash billboard half-size on the water surface.
const SPLASH_HALF_SIZE: f32 = 8.0;

const SPLASH_LIFETIME: f32 = 0.6;

/// Lift above water to avoid z-fighting with the water plane.
const SPLASH_Z_OFFSET: f32 = 2.0;

// -- GPU vertex --

/// GPU vertex for a particle billboard corner.
#[repr(C)]
#[derive(Copy, Clone, Debug, bytemuck::Pod, bytemuck::Zeroable)]
pub struct ParticleVertex {
    pub position: [f32; 3],
    pub alpha: f32,
}

impl ParticleVertex {
    pub fn desc() -> wgpu::VertexBufferLayout<'static> {
        const ATTRS: &[wgpu::VertexAttribute] = &[
            wgpu::VertexAttribute {
                offset: 0,
                shader_location: 0,
                format: wgpu::VertexFormat::Float32x3,
            },
            wgpu::VertexAttribute {
                offset: 12,
                shader_location: 1,
                format: wgpu::VertexFormat::Float32,
            },
        ];

        wgpu::VertexBufferLayout {
            array_stride: size_of::<ParticleVertex>() as wgpu::BufferAddress,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: ATTRS,
        }
    }
}

// -- Internal types --

struct Particle {
    position: Vec3,
    velocity: Vec3,
}

struct SplashEffect {
    position: Vec3,
    age: f32,
}

/// CPU-side weather particle simulation.
pub struct ParticleSystem {
    particles: Vec<Particle>,
    splashes: Vec<SplashEffect>,
    weather: Weather,
    spawn_accum: f32,
    rng_state: u32,
    /// Reused each frame: grows to peak particle count once, then
    /// `build_mesh_into` repopulates in place so steady-state weather
    /// doesn't allocate.
    vertex_scratch: Vec<ParticleVertex>,
    /// Same reuse pattern as `vertex_scratch`.
    index_scratch: Vec<u32>,
}

impl ParticleSystem {
    /// Create an inactive particle system.
    pub fn new() -> Self {
        Self {
            particles: Vec::with_capacity(MAX_PARTICLES),
            splashes: Vec::with_capacity(MAX_SPLASHES),
            weather: Weather::Default,
            spawn_accum: 0.0,
            rng_state: 0xDEAD_BEEF, // xorshift32 only requires non-zero.
            vertex_scratch: Vec::new(),
            index_scratch: Vec::new(),
        }
    }

    /// Set the active weather type, clearing particles on change.
    pub fn set_weather(&mut self, weather: Weather) {
        if weather != self.weather {
            self.weather = weather;
            self.particles.clear();
            self.splashes.clear();
            self.spawn_accum = 0.0;
        }
    }

    /// Advance the simulation by `dt` seconds. When `map` and
    /// `terrain_types` are supplied, particles collide with terrain and
    /// rain spawns splashes on water tiles.
    pub fn update(
        &mut self,
        dt: f32,
        camera_pos: Vec3,
        map: Option<&MapData>,
        terrain_types: Option<&TerrainTypeData>,
    ) {
        if matches!(self.weather, Weather::Default | Weather::Clear) {
            self.particles.clear();
            self.splashes.clear();
            return;
        }

        let cam_x = camera_pos.x;
        let cam_z = camera_pos.z;

        let spawn_rate = match self.weather {
            Weather::Rain => RAIN_SPAWN_RATE,
            Weather::Snow => SNOW_SPAWN_RATE,
            _ => 0.0,
        };
        self.spawn_accum += spawn_rate * dt;
        let to_spawn = self.spawn_accum as usize;
        self.spawn_accum -= to_spawn as f32;

        for _ in 0..to_spawn {
            if self.particles.len() >= MAX_PARTICLES {
                break;
            }
            self.spawn_one(cam_x, cam_z);
        }

        let is_rain = self.weather == Weather::Rain;
        let mut rng = self.rng_state;
        let mut new_splashes = Vec::new();

        self.particles.retain_mut(|p| {
            p.position += p.velocity * dt;

            // ~1/30 chance per update of a snow drift perturbation.
            if !is_rain {
                rng ^= rng << 13;
                rng ^= rng >> 17;
                rng ^= rng << 5;
                if rng.is_multiple_of(30) {
                    p.velocity.x = (xorshift_f32(&mut rng) - 0.5) * SNOW_DRIFT * 2.0;
                    p.velocity.z = (xorshift_f32(&mut rng) - 0.5) * SNOW_DRIFT * 2.0;
                }
            }

            // Skip terrain sampling unless the particle could plausibly hit ground.
            if let Some(map) = map {
                if p.position.y < TILE_MAX_HEIGHT {
                    let ground = sample_terrain_height_pub(map, p.position.x, p.position.z);
                    if p.position.y <= ground || p.position.y < 0.0 {
                        if is_rain && let Some(ttp) = terrain_types {
                            let tx = (p.position.x / TILE_UNITS) as u32;
                            let tz = (p.position.z / TILE_UNITS) as u32;
                            if is_water_tile(map, ttp, tx, tz) {
                                new_splashes.push(Vec3::new(p.position.x, ground, p.position.z));
                            }
                        }
                        return false;
                    }
                }
            } else if p.position.y < 0.0 {
                return false;
            }

            let dx = p.position.x - cam_x;
            let dz = p.position.z - cam_z;
            (dx * dx + dz * dz) < CULL_RADIUS_SQ
        });

        self.rng_state = rng;

        for pos in new_splashes {
            if self.splashes.len() < MAX_SPLASHES {
                self.splashes.push(SplashEffect {
                    position: pos,
                    age: 0.0,
                });
            }
        }

        self.splashes.retain_mut(|s| {
            s.age += dt;
            s.age < SPLASH_LIFETIME
        });
    }

    /// Populate billboard scratch and return (vertices, indices) slices
    /// to upload. Scratch is retained so the steady-state tick path does
    /// not allocate.
    pub fn build_mesh_into(
        &mut self,
        camera_right: Vec3,
        camera_up: Vec3,
    ) -> (&[ParticleVertex], &[u32]) {
        self.vertex_scratch.clear();
        self.index_scratch.clear();

        let particle_count = self.particles.len();
        let splash_count = self.splashes.len();
        if particle_count == 0 && splash_count == 0 {
            return (&self.vertex_scratch, &self.index_scratch);
        }

        let total = particle_count + splash_count;
        self.vertex_scratch.reserve(total * 4);
        self.index_scratch.reserve(total * 6);

        if particle_count > 0 {
            let (half_w, half_h) = match self.weather {
                Weather::Rain => (RAIN_HALF_W, RAIN_HALF_H),
                Weather::Snow => (SNOW_HALF_W, SNOW_HALF_H),
                _ => (0.0, 0.0),
            };

            // Rain streaks are world-vertical; snow billboards face the camera.
            let right = camera_right * half_w;
            let up = if self.weather == Weather::Rain {
                Vec3::Y * half_h
            } else {
                camera_up * half_h
            };

            // WZ2100 uses WZCOL_WHITE with per-texture alpha. Approximate
            // with a fixed value: rain subtle, snow more opaque.
            let alpha = match self.weather {
                Weather::Rain => 0.35,
                Weather::Snow => 0.6,
                _ => 0.0,
            };

            for p in &self.particles {
                push_quad(
                    &mut self.vertex_scratch,
                    &mut self.index_scratch,
                    p.position,
                    right,
                    up,
                    alpha,
                );
            }
        }

        // Two crossed horizontal quads form a rounder splash than one.
        for s in &self.splashes {
            let t = s.age / SPLASH_LIFETIME;
            // Ripple grows 30%->100% size while fading 50%->0% alpha.
            let alpha = 0.5 * (1.0 - t);
            let size = SPLASH_HALF_SIZE * (0.3 + t * 0.7);
            let pos = s.position + Vec3::Y * SPLASH_Z_OFFSET;
            let diag = std::f32::consts::FRAC_1_SQRT_2;
            push_quad(
                &mut self.vertex_scratch,
                &mut self.index_scratch,
                pos,
                Vec3::X * size,
                Vec3::Z * size,
                alpha,
            );
            push_quad(
                &mut self.vertex_scratch,
                &mut self.index_scratch,
                pos,
                Vec3::new(diag, 0.0, diag) * size,
                Vec3::new(-diag, 0.0, diag) * size,
                alpha,
            );
        }

        (&self.vertex_scratch, &self.index_scratch)
    }

    fn spawn_one(&mut self, cam_x: f32, cam_z: f32) {
        let x = cam_x + (self.rand_f32() - 0.5) * SPAWN_RADIUS * 2.0;
        let z = cam_z + (self.rand_f32() - 0.5) * SPAWN_RADIUS * 2.0;

        // Absolute Y per WZ2100 (`pos.y = 1000`); not terrain-relative.
        let y = SPAWN_Y;

        let velocity = match self.weather {
            Weather::Rain => Vec3::new(
                self.rand_f32() * RAIN_DRIFT,
                -(RAIN_FALL_MIN + self.rand_f32() * (RAIN_FALL_MAX - RAIN_FALL_MIN)),
                self.rand_f32() * RAIN_DRIFT,
            ),
            Weather::Snow => Vec3::new(
                (self.rand_f32() - 0.5) * SNOW_DRIFT * 2.0,
                -(SNOW_FALL_MIN + self.rand_f32() * (SNOW_FALL_MAX - SNOW_FALL_MIN)),
                (self.rand_f32() - 0.5) * SNOW_DRIFT * 2.0,
            ),
            _ => Vec3::ZERO,
        };

        self.particles.push(Particle {
            position: Vec3::new(x, y, z),
            velocity,
        });
    }

    fn rand_f32(&mut self) -> f32 {
        xorshift_f32(&mut self.rng_state)
    }
}

fn push_quad(
    vertices: &mut Vec<ParticleVertex>,
    indices: &mut Vec<u32>,
    center: Vec3,
    right: Vec3,
    up: Vec3,
    alpha: f32,
) {
    let base = vertices.len() as u32;
    vertices.push(ParticleVertex {
        position: (center - right - up).into(),
        alpha,
    });
    vertices.push(ParticleVertex {
        position: (center + right - up).into(),
        alpha,
    });
    vertices.push(ParticleVertex {
        position: (center + right + up).into(),
        alpha,
    });
    vertices.push(ParticleVertex {
        position: (center - right + up).into(),
        alpha,
    });
    indices.extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
}

/// Advance xorshift32 state and return a float in [0, 1).
fn xorshift_f32(state: &mut u32) -> f32 {
    *state ^= *state << 13;
    *state ^= *state >> 17;
    *state ^= *state << 5;
    // Mask to lower 24 bits (2^24 = 16,777,216) for a uniform float in [0, 1).
    (*state & 0x00FF_FFFF) as f32 / 16_777_216.0
}

#[cfg(test)]
mod tests {
    use super::*;
    use wz_maplib::terrain_types::TerrainType;

    /// Build a flat map where every tile is at the given height.
    fn flat_map(w: u32, h: u32, height: u16) -> MapData {
        let mut map = MapData::new(w, h);
        for tile in &mut map.tiles {
            tile.height = height;
        }
        map
    }

    /// Build terrain types with a single water tile at (0,0).
    fn ttp_with_water() -> TerrainTypeData {
        // Texture 0 → Water, everything else → Sand.
        TerrainTypeData {
            terrain_types: vec![TerrainType::Water, TerrainType::SandYellow],
        }
    }

    #[test]
    fn build_mesh_into_reuses_scratch_capacity() {
        let mut sys = ParticleSystem::new();
        sys.set_weather(Weather::Snow);
        // Spawn enough particles to force the scratch buffers to grow.
        for _ in 0..10 {
            sys.update(0.1, Vec3::new(500.0, 500.0, 500.0), None, None);
        }
        assert!(
            !sys.particles.is_empty(),
            "snow update should have spawned particles"
        );

        let (v1_len, i1_len) = {
            let (v, i) = sys.build_mesh_into(Vec3::X, Vec3::Y);
            (v.len(), i.len())
        };
        let cap_v_after_first = sys.vertex_scratch.capacity();
        let cap_i_after_first = sys.index_scratch.capacity();
        assert!(v1_len > 0 && i1_len > 0);

        // A second build with an identical particle count must not grow
        // the scratch buffers - repeated rain/snow ticks should be
        // allocation-free under steady state.
        let _ = sys.build_mesh_into(Vec3::X, Vec3::Y);
        assert_eq!(
            sys.vertex_scratch.capacity(),
            cap_v_after_first,
            "vertex scratch should not reallocate under steady state"
        );
        assert_eq!(
            sys.index_scratch.capacity(),
            cap_i_after_first,
            "index scratch should not reallocate under steady state"
        );
    }

    #[test]
    fn clear_weather_removes_all_particles() {
        let mut sys = ParticleSystem::new();
        sys.set_weather(Weather::Rain);
        // Spawn some particles.
        sys.update(0.1, Vec3::new(500.0, 500.0, 500.0), None, None);
        assert!(
            !sys.particles.is_empty(),
            "rain should have spawned particles"
        );

        // Switching to Clear should remove them all.
        sys.set_weather(Weather::Clear);
        assert!(sys.particles.is_empty());
        assert!(sys.splashes.is_empty());
    }

    #[test]
    fn default_weather_produces_no_particles() {
        let mut sys = ParticleSystem::new();
        sys.set_weather(Weather::Default);
        sys.update(1.0, Vec3::ZERO, None, None);
        assert!(sys.particles.is_empty());
    }

    #[test]
    fn rain_hits_ground_and_is_removed() {
        let map = flat_map(8, 8, 0); // Flat terrain at height 0.

        let mut sys = ParticleSystem::new();
        sys.set_weather(Weather::Rain);

        // Spawn particles with a large dt to get many at once.
        sys.update(0.5, Vec3::new(512.0, 500.0, 512.0), Some(&map), None);
        assert!(!sys.particles.is_empty(), "should have spawned rain");

        // Manually place a particle just above ground so it hits next frame.
        sys.particles.push(Particle {
            position: Vec3::new(512.0, 1.0, 512.0),
            velocity: Vec3::new(0.0, -3000.0, 0.0),
        });
        let count_before = sys.particles.len();

        // One frame: the low particle should hit ground and be removed.
        sys.update(1.0 / 60.0, Vec3::new(512.0, 500.0, 512.0), Some(&map), None);

        assert!(
            sys.particles.len() < count_before,
            "particle at ground level should be removed on contact"
        );
    }

    #[test]
    fn rain_on_water_spawns_splashes() {
        // Map where tile (0,0) texture=0 → Water via our TTP.
        let mut map = flat_map(8, 8, 0);
        // Set tile (0,0) to texture 0 (Water).
        map.tiles[0].texture = 0;
        // Set all other tiles to texture 1 (Sand).
        for tile in map.tiles.iter_mut().skip(1) {
            tile.texture = 1;
        }
        let ttp = ttp_with_water();

        let mut sys = ParticleSystem::new();
        sys.set_weather(Weather::Rain);

        // Spawn near tile (0,0) center = (64, ?, 64).
        // Run enough frames for rain to reach the ground.
        for _ in 0..300 {
            sys.update(
                1.0 / 60.0,
                Vec3::new(64.0, 500.0, 64.0),
                Some(&map),
                Some(&ttp),
            );
        }

        // At least some splashes should have been created on the water tile.
        assert!(
            !sys.splashes.is_empty(),
            "rain hitting water should spawn splash effects"
        );
    }

    #[test]
    fn snow_particles_have_drift_perturbation() {
        let mut sys = ParticleSystem::new();
        sys.set_weather(Weather::Snow);

        // Spawn snow.
        sys.update(0.1, Vec3::new(500.0, 500.0, 500.0), None, None);
        assert!(!sys.particles.is_empty());

        // Record initial X velocities.
        let initial_vx: Vec<f32> = sys.particles.iter().map(|p| p.velocity.x).collect();

        // Run many frames - snow drift should change at least one particle's velocity.
        for _ in 0..200 {
            sys.update(1.0 / 60.0, Vec3::new(500.0, 500.0, 500.0), None, None);
        }

        let changed = sys
            .particles
            .iter()
            .zip(initial_vx.iter())
            .any(|(p, &orig)| (p.velocity.x - orig).abs() > 0.01);
        assert!(changed, "snow drift should perturb velocities over time");
    }
}
