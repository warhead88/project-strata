use std::sync::Arc;

use bevy::prelude::*;
use strata_globe::{CameraViewF64, EcefMeters, ElevationSampler, FlatElevation, PlanetSpec};
use strata_terrain::TerrainWorld;

/// Client-side terrain relief sampler resource with monotonic revision tracking
/// so changing terrain invalidates cached GPU tile meshes cleanly.
#[derive(Resource, Clone)]
pub struct TerrainElevation {
    sampler: Arc<dyn ElevationSampler>,
    world: Option<Arc<TerrainWorld>>,
    revision: u64,
}

impl TerrainElevation {
    /// Creates a [`TerrainElevation`] wrapping `sampler` at revision `0`.
    #[must_use]
    pub fn new(sampler: Arc<dyn ElevationSampler>) -> Self {
        Self {
            sampler,
            world: None,
            revision: 0,
        }
    }

    /// Creates a [`TerrainElevation`] backed by a full physical-geographic [`TerrainWorld`].
    #[must_use]
    pub fn from_world(world: Arc<TerrainWorld>) -> Self {
        let sampler: Arc<dyn ElevationSampler> = world.clone();
        Self {
            sampler,
            world: Some(world),
            revision: 0,
        }
    }

    /// Creates a [`TerrainElevation`] initialized with [`TerrainWorld::earth_default`].
    #[must_use]
    pub fn earth_default() -> Self {
        Self::from_world(Arc::new(TerrainWorld::earth_default()))
    }

    /// Active terrain elevation sampler.
    #[must_use]
    pub fn sampler(&self) -> &dyn ElevationSampler {
        self.sampler.as_ref()
    }

    /// Cloned [`Arc<dyn ElevationSampler>`] handle for background mesh generation tasks.
    #[must_use]
    pub(crate) fn sampler_arc(&self) -> Arc<dyn ElevationSampler> {
        Arc::clone(&self.sampler)
    }

    /// Active [`TerrainWorld`] if this terrain carries full hydrology, climate, and biome data.
    #[must_use]
    pub fn world(&self) -> Option<&Arc<TerrainWorld>> {
        self.world.as_ref()
    }

    /// Cloned optional [`Arc<TerrainWorld>`] handle for background mesh generation tasks.
    #[must_use]
    pub(crate) fn world_arc(&self) -> Option<Arc<TerrainWorld>> {
        self.world.as_ref().map(Arc::clone)
    }

    /// Monotonic revision incremented whenever [`Self::set_sampler`] or [`Self::set_world`] replaces the terrain.
    #[must_use]
    pub const fn revision(&self) -> u64 {
        self.revision
    }

    /// Replaces the active elevation sampler (clearing any [`TerrainWorld`]) and bumps the revision.
    #[cfg_attr(
        not(test),
        expect(dead_code, reason = "runtime terrain-switching API exercised by tests")
    )]
    pub fn set_sampler(&mut self, sampler: Arc<dyn ElevationSampler>) {
        self.sampler = sampler;
        self.world = None;
        self.revision = self.revision.wrapping_add(1);
    }

    /// Replaces the active terrain with `world` and bumps the cache-invalidation revision.
    #[cfg_attr(
        not(test),
        expect(dead_code, reason = "runtime terrain-switching API exercised by tests")
    )]
    pub fn set_world(&mut self, world: Arc<TerrainWorld>) {
        self.sampler = world.clone();
        self.world = Some(world);
        self.revision = self.revision.wrapping_add(1);
    }
}

impl Default for TerrainElevation {
    fn default() -> Self {
        Self::new(Arc::new(FlatElevation))
    }
}

/// Samples `sampler` at `unit_dir`, returning finite elevation clamped so `R + elev > 0`.
#[must_use]
pub(crate) fn sample_clamped_elevation(
    planet: PlanetSpec,
    unit_dir: EcefMeters,
    sampler: &dyn ElevationSampler,
) -> f64 {
    if sampler.global_range().is_flat() {
        return 0.0;
    }
    let r = planet.radius_meters();
    let min_r = 1.0_f64.min(r * 0.5);
    let elev = sampler.sample_elevation(unit_dir);
    if elev.is_finite() {
        elev.max(min_r - r)
    } else {
        0.0
    }
}

fn flat_sphere_hit(
    planet: PlanetSpec,
    cam_view: &CameraViewF64,
    ray_dir: EcefMeters,
) -> Option<(EcefMeters, f64)> {
    let r = planet.radius_meters();
    let b = cam_view.position.dot(ray_dir);
    let c = cam_view.position.dot(cam_view.position) - r * r;
    let disc = b * b - c;
    if disc < 0.0 {
        return None;
    }
    let t_hit = -b - disc.sqrt();
    if t_hit <= 0.0 {
        return None;
    }
    let hit_unit = (cam_view.position + ray_dir.scale(t_hit))
        .normalized()
        .ok()?;
    let incidence_cos = (-ray_dir.dot(hit_unit)).max(0.0);
    Some((hit_unit, incidence_cos))
}

fn march_terrain_hit(
    planet: PlanetSpec,
    cam_view: &CameraViewF64,
    ray_dir: EcefMeters,
    t_start: f64,
    t_end: f64,
    sampler: &dyn ElevationSampler,
) -> Option<(EcefMeters, f64)> {
    const MARCH_STEPS: u32 = 512;
    let r_base = planet.radius_meters();
    let min_r = 1.0_f64.min(r_base * 0.5);

    let signed_height = |t: f64| -> f64 {
        let p = cam_view.position + ray_dir.scale(t);
        let p_r = p.norm();
        let Ok(unit) = p.normalized() else {
            return -r_base;
        };
        let surf_r = (r_base + sample_clamped_elevation(planet, unit, sampler)).max(min_r);
        p_r - surf_r
    };

    if signed_height(t_start) <= 0.0 {
        if t_start > 0.0 {
            let hit_unit = (cam_view.position + ray_dir.scale(t_start))
                .normalized()
                .ok()?;
            let incidence_cos = (-ray_dir.dot(hit_unit)).max(0.0);
            return Some((hit_unit, incidence_cos));
        }
        return None;
    }

    let mut t_prev = t_start;
    let span = t_end - t_start;
    let inv_steps = 1.0 / f64::from(MARCH_STEPS);

    let mut bracket: Option<(f64, f64)> = None;
    for step in 1..=MARCH_STEPS {
        let t_curr = t_start + span * (f64::from(step) * inv_steps);
        if signed_height(t_curr) <= 0.0 {
            bracket = Some((t_prev, t_curr));
            break;
        }
        t_prev = t_curr;
    }

    let (mut lo, mut hi) = bracket?;
    for _ in 0..64 {
        if hi - lo <= 1e-6 {
            break;
        }
        let mid = 0.5 * (lo + hi);
        if signed_height(mid) > 0.0 {
            lo = mid;
        } else {
            hi = mid;
        }
    }

    let t_hit = 0.5 * (lo + hi);
    let hit_unit = (cam_view.position + ray_dir.scale(t_hit))
        .normalized()
        .ok()?;
    let incidence_cos = (-ray_dir.dot(hit_unit)).max(0.0);
    Some((hit_unit, incidence_cos))
}

/// Intersects a screen ray at `(ndc_x, ndc_y)` with the displaced terrain surface.
///
/// Uses an exact analytic sphere intersection when `sampler.global_range().is_flat()`.
/// Otherwise intersects the outer sphere `R + max_meters`, marches along the ray up
/// to `R + min_meters` (or outer sphere exit for grazing rays), and bisects the first
/// sign crossing to `< 1e-6 m` precision.
#[must_use]
pub(crate) fn ray_surface_hit_impl(
    planet: PlanetSpec,
    cam_view: &CameraViewF64,
    ndc_x: f64,
    ndc_y: f64,
    sampler: &dyn ElevationSampler,
) -> Option<(EcefMeters, f64)> {
    if !ndc_x.is_finite() || !ndc_y.is_finite() {
        return None;
    }
    let half_tan_y = (cam_view.fov_y_rad * 0.5).tan();
    let half_tan_x = half_tan_y * cam_view.aspect;
    let ray_dir = (cam_view.forward
        + cam_view.right.scale(ndc_x * half_tan_x)
        + cam_view.up.scale(ndc_y * half_tan_y))
    .normalized()
    .ok()?;

    let range = sampler.global_range().clamp_planet_radius(planet);
    if range.is_flat() {
        return flat_sphere_hit(planet, cam_view, ray_dir);
    }

    let r_max = range.max_radius_meters(planet);
    let r_min = range.min_radius_meters(planet);
    let b = cam_view.position.dot(ray_dir);
    let pos_sq = cam_view.position.dot(cam_view.position);
    let disc_max = b * b - (pos_sq - r_max * r_max);
    if disc_max < 0.0 {
        return None;
    }
    let sqrt_max = disc_max.sqrt();
    let t_enter_max = -b - sqrt_max;
    let t_exit_max = -b + sqrt_max;
    if t_exit_max <= 0.0 {
        return None;
    }

    let t_start = t_enter_max.max(0.0);
    let disc_min = b * b - (pos_sq - r_min * r_min);
    let t_end = if disc_min >= 0.0 {
        let t_enter_min = -b - disc_min.sqrt();
        if t_enter_min > t_start {
            t_enter_min
        } else {
            t_exit_max
        }
    } else {
        t_exit_max
    };
    if t_end <= t_start {
        return None;
    }

    march_terrain_hit(planet, cam_view, ray_dir, t_start, t_end, sampler)
}

/// Samples terrain elevations beneath the camera focus point and eye position.
pub(crate) fn sync_terrain_elevations_impl(
    cam: &mut crate::camera::GlobeCameraState,
    sampler: &dyn ElevationSampler,
) {
    if sampler.global_range().is_flat() {
        cam.focus_elevation_m = 0.0;
        cam.eye_ground_elevation_m = 0.0;
        return;
    }
    cam.focus_elevation_m = sample_clamped_elevation(cam.planet, cam.focus_unit, sampler);
    let target = cam.focus_ecef();
    let (sin_p, cos_p) = cam.effective_pitch_rad().sin_cos();
    let view_back = cam.focus_unit.scale(cos_p) - cam.heading_unit.scale(sin_p);
    let provisional_eye = target + view_back.scale(cam.altitude_meters);
    if let Ok(eye_unit) = provisional_eye.normalized() {
        cam.eye_ground_elevation_m = sample_clamped_elevation(cam.planet, eye_unit, sampler);
    } else {
        cam.eye_ground_elevation_m = cam.focus_elevation_m;
    }
}

/// Re-aligns camera orientation so the active zoom cursor anchor remains pixel-locked
/// on displaced terrain after an altitude step.
pub(crate) fn apply_zoom_to_cursor_impl(
    cam: &mut crate::camera::GlobeCameraState,
    aspect: f64,
    viewport_height_px: f64,
    sampler: &dyn ElevationSampler,
) {
    let Some(anchor) = cam.zoom_anchor else {
        return;
    };
    let is_flat = sampler.global_range().is_flat();
    let max_iters = if is_flat { 1 } else { 8 };
    let mut prev_err = f64::INFINITY;

    for _ in 0..max_iters {
        if !is_flat {
            sync_terrain_elevations_impl(cam, sampler);
        }
        let Ok(cam_view) = cam.to_camera_view_f64(aspect, viewport_height_px) else {
            return;
        };
        let Some((after_unit, _)) =
            cam.ray_surface_hit(&cam_view, anchor.ndc_x, anchor.ndc_y, sampler)
        else {
            cam.zoom_anchor = None;
            return;
        };
        let err = after_unit.distance_to(anchor.world_unit) * cam.planet.radius_meters();
        if err <= 1e-4 || err >= prev_err {
            break;
        }
        prev_err = err;
        if !cam.rotate_sphere_from_to(after_unit, anchor.world_unit, 0.75) {
            break;
        }
    }
    if !is_flat {
        sync_terrain_elevations_impl(cam, sampler);
    }
}
