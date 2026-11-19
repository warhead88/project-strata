use bevy::prelude::*;
use strata_globe::{CameraViewF64, EcefMeters, GlobeError, LocalOrigin, PlanetSpec, RebasePolicy};

/// Minimum camera altitude above planet surface in meters.
pub const MIN_ALTITUDE_METERS: f64 = 20.0;

/// Maximum orbital altitude factor relative to planet radius (`~20,000 km` on Earth).
pub const MAX_ALTITUDE_RADIUS_FACTOR: f64 = 3.2;

/// Marker component for the primary 3D globe camera.
#[derive(Component)]
pub struct GlobeCamera;

/// Authoritative `f64` orbital-to-local camera state.
///
/// Never derived from `f32` Bevy [`Transform`]s; instead drives Bevy's camera
/// transform and projection after subtracting [`LocalOrigin`].
#[derive(Resource, Debug, Clone)]
pub struct GlobeCameraState {
    /// Unit direction from planet center to the surface focus point.
    pub focus_unit: EcefMeters,
    /// Current damped altitude above sphere surface in meters.
    pub altitude_meters: f64,
    /// Target altitude above sphere surface in meters (logarithmically zoomed).
    pub target_altitude_meters: f64,
    /// Camera azimuth around the local surface normal in radians.
    pub yaw_rad: f64,
    /// Camera tilt away from nadir (`0` = straight down, `1.25` = oblique horizon).
    pub pitch_rad: f64,
    /// Vertical field of view in radians.
    pub fov_y_rad: f64,
    /// Active planet geometry specification.
    pub planet: PlanetSpec,
}

impl GlobeCameraState {
    /// Initializes camera framing a focus direction at orbital overview altitude.
    ///
    /// # Errors
    ///
    /// Returns [`GlobeError`] if `focus_direction` cannot be normalized.
    pub fn new(planet: PlanetSpec, focus_direction: EcefMeters) -> Result<Self, GlobeError> {
        let focus_unit = focus_direction.normalized()?;
        let initial_alt = (planet.radius_meters() * 2.5).max(MIN_ALTITUDE_METERS);
        Ok(Self {
            focus_unit,
            altitude_meters: initial_alt,
            target_altitude_meters: initial_alt,
            yaw_rad: 0.35,
            pitch_rad: 0.45,
            fov_y_rad: 60.0_f64.to_radians(),
            planet,
        })
    }

    /// Maximum allowed orbital altitude for this [`PlanetSpec`].
    #[must_use]
    pub fn max_altitude_meters(&self) -> f64 {
        (self.planet.radius_meters() * MAX_ALTITUDE_RADIUS_FACTOR).max(1_000.0)
    }

    /// Surface focus point in `f64` ECEF meters.
    #[must_use]
    pub fn focus_ecef(&self) -> EcefMeters {
        self.focus_unit.scale(self.planet.radius_meters())
    }

    /// Moves the focus point smoothly to frame `point_ecef` without mutating `point_ecef`.
    pub fn focus_on_point(&mut self, point_ecef: EcefMeters, altitude: Option<f64>) {
        if let Ok(unit) = point_ecef.normalized() {
            self.focus_unit = unit;
        }
        if let Some(alt) = altitude {
            self.target_altitude_meters =
                alt.clamp(MIN_ALTITUDE_METERS, self.max_altitude_meters());
        }
    }

    /// Pans the surface focus point along the camera's tangent `(right, forward)` axes.
    pub fn pan_tangent(&mut self, delta_right_rad: f64, delta_forward_rad: f64) {
        let (_east, _north, cam_right, cam_heading) = self.local_tangent_frame();
        let rotated = self.focus_unit
            + cam_right.scale(delta_right_rad)
            + cam_heading.scale(delta_forward_rad);
        if let Ok(unit) = rotated.normalized() {
            self.focus_unit = unit;
        }
    }

    /// Computes `(eye_ecef, look_target_ecef, up_hint_ecef)` in `f64` with
    /// pole-safe surface-tangent up alignment.
    #[must_use]
    pub fn compute_pose_f64(&self) -> (EcefMeters, EcefMeters, EcefMeters) {
        let normal = self.focus_unit;
        let (_east, _north, _right, heading) = self.local_tangent_frame();
        let target = self.focus_ecef();
        let (sin_p, cos_p) = self.pitch_rad.sin_cos();

        let view_back = normal.scale(cos_p) - heading.scale(sin_p);
        let up_hint = normal.scale(sin_p) + heading.scale(cos_p);
        let mut eye = target + view_back.scale(self.altitude_meters);

        // Keep camera safely above the surface sphere at grazing pitch angles.
        let min_r = self.planet.radius_meters() + 5.0;
        let eye_r = eye.norm();
        if eye_r < min_r && eye_r > 0.0 {
            eye = eye.scale(min_r / eye_r);
        }

        (eye, target, up_hint)
    }

    /// Builds a [`CameraViewF64`] for pure-Rust LOD selection and culling.
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

    fn local_tangent_frame(&self) -> (EcefMeters, EcefMeters, EcefMeters, EcefMeters) {
        let normal = self.focus_unit;
        let ref_axis = if normal.y.abs() < 0.99 {
            EcefMeters::from_raw(0.0, 1.0, 0.0)
        } else {
            EcefMeters::from_raw(0.0, 0.0, 1.0)
        };
        let east = ref_axis
            .cross(normal)
            .normalized()
            .unwrap_or(EcefMeters::from_raw(1.0, 0.0, 0.0));
        let north = normal
            .cross(east)
            .normalized()
            .unwrap_or(EcefMeters::from_raw(0.0, 1.0, 0.0));

        let (sin_y, cos_y) = self.yaw_rad.sin_cos();
        let heading = north.scale(cos_y) + east.scale(sin_y);
        let right = heading.cross(normal).normalized().unwrap_or(east);
        (east, north, right, heading)
    }
}

/// Floating-origin state maintained by the client render layer.
#[derive(Resource, Debug, Clone)]
pub struct OriginState {
    /// Current `f64` floating-origin anchor near the camera.
    pub origin: LocalOrigin,
    /// Hysteresis rebase policy.
    pub policy: RebasePolicy,
    /// Total number of origin rebases performed in this session.
    pub rebase_count: u64,
}

impl OriginState {
    /// Initializes [`OriginState`] near `initial_camera_ecef`.
    ///
    /// # Errors
    ///
    /// Returns [`GlobeError`] if `initial_camera_ecef` is non-finite.
    pub fn new(initial_camera_ecef: EcefMeters) -> Result<Self, GlobeError> {
        Ok(Self {
            origin: LocalOrigin::new(initial_camera_ecef)?,
            policy: RebasePolicy::standard(),
            rebase_count: 0,
        })
    }
}

/// Computes dynamic `(near, far)` clip distances (`f32`) scaled to camera altitude
/// and planet radius so both `20 m` local scenes and `20,000 km` orbit views avoid z-fighting.
#[must_use]
#[expect(
    clippy::cast_possible_truncation,
    reason = "perspective clip planes are GPU f32 parameters derived from f64 distances"
)]
pub fn dynamic_clip_planes(altitude_meters: f64, radius_meters: f64) -> (f32, f32) {
    let alt = altitude_meters.max(1.0);
    let near = (alt * 0.02).clamp(0.25, 50_000.0);
    let horizon = ((radius_meters + alt).powi(2) - radius_meters.powi(2))
        .max(0.0)
        .sqrt();
    let far = horizon.max(alt + 2.5 * radius_meters).max(100_000.0);
    (near as f32, far as f32)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn camera_pose_and_clip_planes_stay_valid_across_scales() -> Result<(), GlobeError> {
        let planet = PlanetSpec::earth();
        let mut cam = GlobeCameraState::new(planet, EcefMeters::from_raw(1.0, 1.0, 1.0))?;
        let mut origin_state = OriginState::new(cam.compute_pose_f64().0)?;

        for alt in [20_000_000.0, 500_000.0, 10_000.0, 20.0] {
            cam.altitude_meters = alt;
            cam.target_altitude_meters = alt;
            let (eye, target, up) = cam.compute_pose_f64();
            assert!(eye.norm() > planet.radius_meters());
            assert!((target.norm() - planet.radius_meters()).abs() <= 1e-6);
            assert!((up.norm() - 1.0).abs() <= 1e-9);

            origin_state
                .policy
                .maybe_rebase(&mut origin_state.origin, eye, alt);
            let rel_target = origin_state.origin.to_render_vec3(target);
            if alt <= 5_000.0 {
                let err = origin_state
                    .origin
                    .reconstruct_ecef_f64(rel_target)
                    .distance_to(target);
                assert!(err <= 1e-3);
            }
            let (near, far) = dynamic_clip_planes(alt, planet.radius_meters());
            assert!(near > 0.0 && far > near);
        }
        Ok(())
    }

    #[test]
    fn tangent_pan_keeps_focus_on_unit_sphere() -> Result<(), GlobeError> {
        let mut cam = GlobeCameraState::new(
            PlanetSpec::with_radius(1_000_000.0)?,
            EcefMeters::from_raw(0.0, 1.0, 0.0),
        )?;
        for _ in 0..20 {
            cam.pan_tangent(0.05, 0.03);
            assert!((cam.focus_unit.norm() - 1.0).abs() <= 1e-12);
        }
        Ok(())
    }
}
