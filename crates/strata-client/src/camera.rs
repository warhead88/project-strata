use bevy::prelude::*;
use strata_globe::{CameraViewF64, EcefMeters, GlobeError, PlanetSpec};

/// Minimum camera altitude above planet surface in meters.
pub const MIN_ALTITUDE_METERS: f64 = 20.0;

/// Maximum orbital altitude factor relative to planet radius (`~20,000 km` on Earth).
pub const MAX_ALTITUDE_RADIUS_FACTOR: f64 = 3.2;

/// Altitude-to-radius ratio below which full surface-tilted strategy camera applies (`~95 km` on Earth).
pub const SURFACE_VIEW_ALTITUDE_RATIO: f64 = 0.015;

/// Altitude-to-radius ratio above which camera centers radially on planet center (`~2,867 km` on Earth).
pub const ORBITAL_VIEW_ALTITUDE_RATIO: f64 = 0.45;

/// Maximum surface tilt away from nadir in radians (`~71.6 deg`).
pub const MAX_PITCH_RAD: f64 = 1.25;

/// Duration of the smooth "Reset to North" animation in seconds.
pub const NORTH_RESET_DURATION_SECS: f64 = 0.3;

/// Marker component for the primary 3D globe camera.
#[derive(Component)]
pub struct GlobeCamera;

/// World anchor pinned under the mouse cursor during damped Zoom-to-Cursor.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ZoomCursorAnchor {
    /// Unit direction on the sphere that must stay under `(ndc_x, ndc_y)`.
    pub world_unit: EcefMeters,
    /// Horizontal normalized device coordinate in `[-1, 1]`.
    pub ndc_x: f64,
    /// Vertical normalized device coordinate in `[-1, 1]`.
    pub ndc_y: f64,
}

/// Active smoothstep animation rotating camera heading back to geographic North.
#[derive(Debug, Clone, Copy, PartialEq)]
struct NorthResetAnim {
    elapsed: f64,
    duration: f64,
    prev_smooth: f64,
    total_angle: f64,
}

/// Authoritative `f64` orbital-to-local camera state with a singularity-free
/// parallel-transported tangent basis `(focus_unit, heading_unit)`.
///
/// Never derived from `f32` Bevy [`Transform`]s; instead drives Bevy's camera
/// transform and projection after subtracting [`LocalOrigin`].
#[derive(Resource, Debug, Clone)]
pub struct GlobeCameraState {
    /// Unit outward radial direction from planet center to the surface focus point.
    pub focus_unit: EcefMeters,
    /// Unit tangent direction orthogonal to `focus_unit` pointing toward screen-up.
    pub heading_unit: EcefMeters,
    /// Current damped altitude above sphere surface in meters.
    pub altitude_meters: f64,
    /// Target altitude above sphere surface in meters (logarithmically zoomed).
    pub target_altitude_meters: f64,
    /// Desired local surface tilt away from nadir (`0` = straight down, `MAX_PITCH_RAD` = oblique).
    pub pitch_rad: f64,
    /// Vertical field of view in radians.
    pub fov_y_rad: f64,
    /// Active planet geometry specification.
    pub planet: PlanetSpec,
    /// Active Zoom-to-Cursor anchor while wheel zoom is damping.
    pub zoom_anchor: Option<ZoomCursorAnchor>,
    north_reset: Option<NorthResetAnim>,
}

impl GlobeCameraState {
    /// Initializes camera framing a focus direction at orbital overview altitude (North-up).
    ///
    /// # Errors
    ///
    /// Returns [`GlobeError`] if `focus_direction` cannot be normalized.
    pub fn new(planet: PlanetSpec, focus_direction: EcefMeters) -> Result<Self, GlobeError> {
        let focus_unit = focus_direction.normalized()?;
        let heading_unit = initial_north_tangent(focus_unit);
        let initial_alt = (planet.radius_meters() * 2.5).max(MIN_ALTITUDE_METERS);
        Ok(Self {
            focus_unit,
            heading_unit,
            altitude_meters: initial_alt,
            target_altitude_meters: initial_alt,
            pitch_rad: 0.45,
            fov_y_rad: 60.0_f64.to_radians(),
            planet,
            zoom_anchor: None,
            north_reset: None,
        })
    }

    /// Maximum allowed orbital altitude for this [`PlanetSpec`].
    #[must_use]
    pub fn max_altitude_meters(&self) -> f64 {
        (self.planet.radius_meters() * MAX_ALTITUDE_RADIUS_FACTOR).max(1_000.0)
    }

    /// Smooth blend factor in `[0.0, 1.0]` between local surface camera (`0.0`)
    /// and planet-centered radial orbital camera (`1.0`), computed in log-altitude space.
    #[must_use]
    pub fn orbital_blend(&self) -> f64 {
        let r = self.planet.radius_meters();
        let lo = (r * SURFACE_VIEW_ALTITUDE_RATIO).max(MIN_ALTITUDE_METERS);
        let hi = (r * ORBITAL_VIEW_ALTITUDE_RATIO).max(lo * 2.0);
        let alt = self.altitude_meters.max(MIN_ALTITUDE_METERS);
        let t = ((alt.ln() - lo.ln()) / (hi.ln() - lo.ln())).clamp(0.0, 1.0);
        t * t * (3.0 - 2.0 * t)
    }

    /// Effective tilt angle in radians after blending smoothly to `0` in orbital view
    /// so the globe stays centered on planet center while zooming from orbit.
    #[must_use]
    pub fn effective_pitch_rad(&self) -> f64 {
        self.pitch_rad * (1.0 - self.orbital_blend())
    }

    /// Unit screen-right tangent vector (`heading x focus_unit`).
    #[must_use]
    pub fn right_unit(&self) -> EcefMeters {
        self.heading_unit
            .cross(self.focus_unit)
            .normalized()
            .unwrap_or(EcefMeters::from_raw(1.0, 0.0, 0.0))
    }

    /// Surface focus point in `f64` ECEF meters.
    #[must_use]
    pub fn focus_ecef(&self) -> EcefMeters {
        self.focus_unit.scale(self.planet.radius_meters())
    }

    /// Moves the focus point to frame `point_ecef` with North-up orientation without mutating `point_ecef`.
    pub fn focus_on_point(&mut self, point_ecef: EcefMeters, altitude: Option<f64>) {
        if let Ok(unit) = point_ecef.normalized() {
            self.focus_unit = unit;
            self.heading_unit = initial_north_tangent(unit);
            self.reorthonormalize();
        }
        if let Some(alt) = altitude {
            self.target_altitude_meters =
                alt.clamp(MIN_ALTITUDE_METERS, self.max_altitude_meters());
        }
        self.zoom_anchor = None;
        self.north_reset = None;
    }

    /// Pans the camera across the sphere using exact great-circle parallel transport,
    /// eliminating any pole singularity or flip when crossing North/South poles.
    pub fn pan_tangent(&mut self, delta_right_rad: f64, delta_forward_rad: f64) {
        let right = self.right_unit();
        let step = right.scale(delta_right_rad) + self.heading_unit.scale(delta_forward_rad);
        let angle = step.norm();
        if angle <= 1e-12 || !angle.is_finite() {
            return;
        }
        let dir = step.scale(1.0 / angle);
        if let Ok(axis) = self.focus_unit.cross(dir).normalized() {
            self.rotate_frame(axis, angle);
        }
    }

    /// Rotates camera heading clockwise around the local radial axis (`focus_unit`).
    pub fn rotate_yaw(&mut self, delta_clockwise_rad: f64) {
        if !delta_clockwise_rad.is_finite() || delta_clockwise_rad == 0.0 {
            return;
        }
        self.north_reset = None;
        self.heading_unit =
            rotate_around_axis(self.heading_unit, self.focus_unit, -delta_clockwise_rad);
        self.reorthonormalize();
    }

    /// Adjusts local surface tilt away from nadir, clamped to `[0.0, MAX_PITCH_RAD]`.
    pub fn adjust_pitch(&mut self, delta_rad: f64) {
        if delta_rad.is_finite() {
            self.pitch_rad = (self.pitch_rad + delta_rad).clamp(0.0, MAX_PITCH_RAD);
        }
    }

    /// Rotates both `focus_unit` and `heading_unit` by the minimal great-circle rotation
    /// mapping `from_unit` onto `to_unit` (used by Zoom-to-Cursor and ray-grab panning).
    pub fn rotate_sphere_from_to(
        &mut self,
        from_unit: EcefMeters,
        to_unit: EcefMeters,
        max_angle_rad: f64,
    ) -> bool {
        let dot = from_unit.dot(to_unit).clamp(-1.0, 1.0);
        let angle = dot.acos();
        if !angle.is_finite() || angle <= 1e-12 || angle > max_angle_rad {
            return false;
        }
        let Ok(axis) = from_unit.cross(to_unit).normalized() else {
            return false;
        };
        self.rotate_frame(axis, angle);
        true
    }

    /// Local geographic North tangent direction at `focus_unit`, or `None` directly at a pole.
    #[must_use]
    pub fn geographic_north(&self) -> Option<EcefMeters> {
        let north_axis = EcefMeters::from_raw(0.0, 1.0, 0.0);
        let proj = north_axis - self.focus_unit.scale(north_axis.dot(self.focus_unit));
        if proj.norm() < 1e-6 {
            None
        } else {
            proj.normalized().ok()
        }
    }

    /// Signed angle in `[-pi, +pi]` from screen-up (`heading_unit`) to geographic North
    /// around `focus_unit` (`0.0` when North is straight up on screen).
    #[must_use]
    pub fn compass_north_angle_rad(&self) -> f64 {
        let Some(north) = self.geographic_north() else {
            return 0.0;
        };
        let cos_a = self.heading_unit.dot(north).clamp(-1.0, 1.0);
        let sin_a = self.focus_unit.dot(self.heading_unit.cross(north));
        sin_a.atan2(cos_a)
    }

    /// Starts the smooth `0.3 s` animation returning geographic North to screen-up.
    pub fn start_north_reset(&mut self) {
        let Some(north) = self.geographic_north() else {
            self.north_reset = None;
            return;
        };
        let cos_a = self.heading_unit.dot(north).clamp(-1.0, 1.0);
        let sin_a = self.focus_unit.dot(self.heading_unit.cross(north));
        let total_angle = sin_a.atan2(cos_a);
        if total_angle.abs() <= 1e-6 {
            self.heading_unit = north;
            self.reorthonormalize();
            self.north_reset = None;
        } else {
            self.north_reset = Some(NorthResetAnim {
                elapsed: 0.0,
                duration: NORTH_RESET_DURATION_SECS,
                prev_smooth: 0.0,
                total_angle,
            });
        }
    }

    /// Advances an active "Reset to North" animation by `dt` seconds.
    pub fn step_north_reset(&mut self, dt: f64) {
        let Some(mut anim) = self.north_reset else {
            return;
        };
        anim.elapsed = (anim.elapsed + dt.max(0.0)).min(anim.duration);
        let t = (anim.elapsed / anim.duration).clamp(0.0, 1.0);
        let smooth = t * t * (3.0 - 2.0 * t);
        let delta = anim.total_angle * (smooth - anim.prev_smooth);
        anim.prev_smooth = smooth;

        self.heading_unit = rotate_around_axis(self.heading_unit, self.focus_unit, delta);
        self.reorthonormalize();

        if t >= 1.0 {
            if let Some(north) = self.geographic_north() {
                self.heading_unit = north;
                self.reorthonormalize();
            }
            self.north_reset = None;
        } else {
            self.north_reset = Some(anim);
        }
    }

    /// Computes `(eye_ecef, look_target_ecef, up_hint_ecef)` in `f64`.
    #[must_use]
    pub fn compute_pose_f64(&self) -> (EcefMeters, EcefMeters, EcefMeters) {
        let normal = self.focus_unit;
        let heading = self.heading_unit;
        let target = self.focus_ecef();
        let (sin_p, cos_p) = self.effective_pitch_rad().sin_cos();

        let view_back = normal.scale(cos_p) - heading.scale(sin_p);
        let up_hint = normal.scale(sin_p) + heading.scale(cos_p);
        let mut eye = target + view_back.scale(self.altitude_meters);

        let min_r = self.planet.radius_meters() + 5.0;
        let eye_r = eye.norm();
        if eye_r < min_r && eye_r > 0.0 {
            eye = eye.scale(min_r / eye_r);
        }

        (eye, target, up_hint)
    }

    /// Builds a [`CameraViewF64`] for pure-Rust LOD selection, culling, and raycasting.
    ///
    /// # Errors
    ///
    /// Returns [`GlobeError`] if camera projection parameters are degenerate.
    pub fn to_camera_view_f64(
        &self,
        aspect: f64,
        viewport_height_px: f64,
    ) -> Result<CameraViewF64, GlobeError> {
        let (eye, target, up_hint) = self.compute_pose_f64();
        CameraViewF64::look_at(
            eye,
            target,
            up_hint,
            self.fov_y_rad,
            aspect,
            viewport_height_px,
        )
    }

    /// Intersects a screen ray at `(ndc_x, ndc_y)` with the planet sphere, returning
    /// `(hit_unit_normal, incidence_cos)` where `incidence_cos > 0` faces the camera.
    #[must_use]
    pub fn ray_sphere_hit(
        &self,
        cam_view: &CameraViewF64,
        ndc_x: f64,
        ndc_y: f64,
    ) -> Option<(EcefMeters, f64)> {
        let half_tan_y = (cam_view.fov_y_rad * 0.5).tan();
        let half_tan_x = half_tan_y * cam_view.aspect;
        let ray_dir = (cam_view.forward
            + cam_view.right.scale(ndc_x * half_tan_x)
            + cam_view.up.scale(ndc_y * half_tan_y))
        .normalized()
        .ok()?;
        let r = self.planet.radius_meters();
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

    /// Re-aligns `(focus_unit, heading_unit)` so the active [`ZoomCursorAnchor`] stays
    /// pixel-locked under the cursor after an altitude step.
    pub fn apply_zoom_to_cursor(&mut self, aspect: f64, viewport_height_px: f64) {
        let Some(anchor) = self.zoom_anchor else {
            return;
        };
        let Ok(cam_view) = self.to_camera_view_f64(aspect, viewport_height_px) else {
            return;
        };
        if let Some((after_unit, _)) = self.ray_sphere_hit(&cam_view, anchor.ndc_x, anchor.ndc_y) {
            let _ = self.rotate_sphere_from_to(after_unit, anchor.world_unit, 0.75);
        } else {
            self.zoom_anchor = None;
        }
    }

    fn rotate_frame(&mut self, unit_axis: EcefMeters, angle_rad: f64) {
        self.focus_unit = rotate_around_axis(self.focus_unit, unit_axis, angle_rad);
        self.heading_unit = rotate_around_axis(self.heading_unit, unit_axis, angle_rad);
        self.reorthonormalize();
    }

    fn reorthonormalize(&mut self) {
        if let Ok(n) = self.focus_unit.normalized() {
            self.focus_unit = n;
        }
        let proj = self.heading_unit
            - self
                .focus_unit
                .scale(self.heading_unit.dot(self.focus_unit));
        if let Ok(h) = proj.normalized() {
            self.heading_unit = h;
        } else {
            self.heading_unit = initial_north_tangent(self.focus_unit);
        }
    }
}

pub use crate::camera_sync::OriginState;
#[cfg(test)]
use crate::camera_sync::dynamic_clip_planes;
use crate::camera_sync::{initial_north_tangent, rotate_around_axis};

#[cfg(test)]
#[path = "camera_tests.rs"]
mod tests;
