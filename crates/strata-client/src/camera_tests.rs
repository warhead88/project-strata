use std::f64::consts::PI;
use std::sync::Arc;

use strata_globe::{
    DEFAULT_GRID_SEGMENTS, EcefMeters, ElevationRange, ElevationSampler, GlobeError, PlanetSpec,
};

use super::{
    GlobeCameraState, MIN_ALTITUDE_METERS, NORTH_RESET_DURATION_SECS, OriginState,
    ZoomCursorAnchor, dynamic_clip_planes,
};
use crate::terrain::TerrainElevation;

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
fn continuous_ws_pan_crosses_both_poles_smoothly_without_oscillation() -> Result<(), GlobeError> {
    let mut cam = GlobeCameraState::new(PlanetSpec::earth(), EcefMeters::from_raw(0.0, 0.0, 1.0))?;
    let step = (2.0 * PI) / 360.0;
    let mut prev_focus = cam.focus_unit;
    let mut prev_heading = cam.heading_unit;
    let mut reached_north_pole = false;
    let mut reached_south_pole = false;

    // Full 360-degree polar orbit holding W continuously.
    for _ in 0..360 {
        cam.pan_tangent(0.0, step);
        assert!((cam.focus_unit.norm() - 1.0).abs() <= 1e-12);
        assert!((cam.heading_unit.norm() - 1.0).abs() <= 1e-12);
        assert!(cam.focus_unit.dot(cam.heading_unit).abs() <= 1e-12);

        // No sudden frame flips or reversals across poles.
        assert!(prev_focus.dot(cam.focus_unit) > 0.999);
        assert!(prev_heading.dot(cam.heading_unit) > 0.999);

        if cam.focus_unit.y > 0.999 {
            reached_north_pole = true;
        }
        if cam.focus_unit.y < -0.999 {
            reached_south_pole = true;
        }
        prev_focus = cam.focus_unit;
        prev_heading = cam.heading_unit;
    }

    assert!(reached_north_pole);
    assert!(reached_south_pole);
    // Returns cleanly to starting meridian after 360 degrees.
    assert!(
        cam.focus_unit
            .distance_to(EcefMeters::from_raw(0.0, 0.0, 1.0))
            <= 1e-9
    );
    assert!(
        cam.heading_unit
            .distance_to(EcefMeters::from_raw(0.0, 1.0, 0.0))
            <= 1e-9
    );
    Ok(())
}

#[test]
fn own_axis_yaw_and_north_reset_finish_in_0_3_seconds() -> Result<(), GlobeError> {
    let mut cam = GlobeCameraState::new(PlanetSpec::earth(), EcefMeters::from_raw(0.0, 0.0, 1.0))?;
    let initial_north = cam.heading_unit;

    // Rotate 115 degrees clockwise around local surface normal.
    cam.rotate_yaw(115.0_f64.to_radians());
    assert!((cam.compass_north_angle_rad() - 115.0_f64.to_radians()).abs() <= 1e-9);

    cam.start_north_reset();
    // Halfway through 0.3s animation: halfway back toward North.
    cam.step_north_reset(NORTH_RESET_DURATION_SECS * 0.5);
    assert!(cam.compass_north_angle_rad().abs() < 65.0_f64.to_radians());
    assert!(cam.compass_north_angle_rad().abs() > 50.0_f64.to_radians());

    // After full 0.3s: aligned precisely back to geographic North.
    cam.step_north_reset(NORTH_RESET_DURATION_SECS * 0.5);
    assert!(cam.compass_north_angle_rad().abs() <= 1e-9);
    assert!(cam.heading_unit.distance_to(initial_north) <= 1e-9);
    Ok(())
}

#[test]
fn zoom_to_cursor_keeps_off_center_surface_point_locked() -> Result<(), GlobeError> {
    let planet = PlanetSpec::earth();
    let r = planet.radius_meters();
    let mut cam = GlobeCameraState::new(planet, EcefMeters::from_raw(0.0, 0.0, 1.0))?;
    cam.altitude_meters = r * 2.0;

    let aspect = 16.0 / 9.0;
    let height_px = 1080.0;
    let ndc_x = 0.28;
    let ndc_y = -0.18;

    let view0 = cam.to_camera_view_f64(aspect, height_px)?;
    let Some((world_unit, _)) = cam.ray_sphere_hit(&view0, ndc_x, ndc_y) else {
        return Err(GlobeError::InvalidCameraProjection);
    };
    cam.zoom_anchor = Some(ZoomCursorAnchor {
        world_unit,
        ndc_x,
        ndc_y,
    });

    // Zoom logarithmically all the way from 2*R down to 20 m.
    for step in 1..=25_u32 {
        let t = f64::from(step) / 25.0;
        cam.altitude_meters = (r * 2.0).powf(1.0 - t) * MIN_ALTITUDE_METERS.powf(t);
        cam.apply_zoom_to_cursor(aspect, height_px);

        let view_now = cam.to_camera_view_f64(aspect, height_px)?;
        let Some((rehit, _)) = cam.ray_sphere_hit(&view_now, ndc_x, ndc_y) else {
            return Err(GlobeError::InvalidCameraProjection);
        };
        let err_m = rehit.scale(r).distance_to(world_unit.scale(r));
        assert!(
            err_m <= 1e-3,
            "zoom-to-cursor point drifted {err_m:.3e} m at step {step}"
        );
    }
    Ok(())
}

#[test]
fn orbital_zoom_centers_on_planet_and_transitions_smoothly_to_surface() -> Result<(), GlobeError> {
    for planet in [PlanetSpec::earth(), PlanetSpec::with_radius(1_737_400.0)?] {
        let r = planet.radius_meters();
        let mut cam = GlobeCameraState::new(planet, EcefMeters::from_raw(1.0, 1.0, 1.0))?;
        cam.pitch_rad = 0.6;

        cam.altitude_meters = r * 2.5;
        assert!((cam.orbital_blend() - 1.0).abs() <= 1e-12);
        assert!(cam.effective_pitch_rad().abs() <= 1e-12);
        let (eye_orb, target_orb, _) = cam.compute_pose_f64();
        let to_surface = (target_orb - eye_orb).normalized()?;
        let to_center = (EcefMeters::ZERO - eye_orb).normalized()?;
        assert!(to_surface.distance_to(to_center) <= 1e-12);

        let mut prev_blend = 1.0_f64;
        for step in 0..=30_u32 {
            let t = f64::from(step) / 30.0;
            cam.altitude_meters = (r * 2.5).powf(1.0 - t) * MIN_ALTITUDE_METERS.powf(t);
            let blend = cam.orbital_blend();
            assert!(blend <= prev_blend + 1e-12);
            prev_blend = blend;
        }

        cam.altitude_meters = MIN_ALTITUDE_METERS;
        assert!(cam.orbital_blend().abs() <= 1e-12);
        assert!((cam.effective_pitch_rad() - cam.pitch_rad).abs() <= 1e-12);
    }
    Ok(())
}

struct RidgeSampler {
    range: ElevationRange,
}

impl ElevationSampler for RidgeSampler {
    fn sample_elevation(&self, u: EcefMeters) -> f64 {
        let wave = (u.x * 6.0 + u.y * 4.0).sin() * (u.z * 7.0 - u.x * 2.0).cos();
        let mid = 0.5 * (self.range.min_meters + self.range.max_meters);
        let half = 0.5 * self.range.span_meters();
        mid + half * wave.clamp(-1.0, 1.0)
    }

    fn global_range(&self) -> ElevationRange {
        self.range
    }
}

struct EyePeakSampler {
    peak_dir: EcefMeters,
    cone_radius_unit: f64,
    range: ElevationRange,
}

impl ElevationSampler for EyePeakSampler {
    fn sample_elevation(&self, u: EcefMeters) -> f64 {
        if u.distance_to(self.peak_dir) < self.cone_radius_unit {
            self.range.max_meters
        } else {
            self.range.min_meters
        }
    }

    fn global_range(&self) -> ElevationRange {
        self.range
    }
}

#[test]
fn terrain_camera_clearance_and_focus_surface_follow_elevation() -> Result<(), GlobeError> {
    let planet = PlanetSpec::earth();
    let r = planet.radius_meters();
    let mut cam = GlobeCameraState::new(planet, EcefMeters::from_raw(0.0, 0.0, 1.0))?;
    cam.altitude_meters = 80.0;
    cam.target_altitude_meters = 80.0;
    cam.pitch_rad = 1.2;

    // Determine where provisional eye sits over the surface and place a +9000 m peak there.
    let (sin_p, cos_p) = cam.effective_pitch_rad().sin_cos();
    let view_back = cam.focus_unit.scale(cos_p) - cam.heading_unit.scale(sin_p);
    let prov_eye = (cam.focus_ecef() + view_back.scale(cam.altitude_meters)).normalized()?;
    let cone_radius_unit = 0.4 * cam.focus_unit.distance_to(prov_eye);

    let sampler = EyePeakSampler {
        peak_dir: prov_eye,
        cone_radius_unit,
        range: ElevationRange::new(-500.0, 9_000.0)?,
    };
    cam.sync_terrain_elevations(&sampler);

    assert!((cam.focus_elevation_m - (-500.0)).abs() <= 1e-9);
    assert!((cam.eye_ground_elevation_m - 9_000.0).abs() <= 1e-9);
    assert!((cam.focus_ecef().norm() - (r - 500.0)).abs() <= 1e-6);

    let (eye, target, _) = cam.compute_pose_f64();
    assert!((target.norm() - (r - 500.0)).abs() <= 1e-6);
    assert!(eye.norm() >= r + cam.eye_ground_elevation_m + 5.0 - 1e-9);
    Ok(())
}

#[test]
fn terrain_raycast_and_zoom_to_cursor_work_on_earth_and_small_planet() -> Result<(), GlobeError> {
    let cases = [
        (
            PlanetSpec::earth(),
            ElevationRange::new(-10_000.0, 9_000.0)?,
            1e-4,
        ),
        (
            PlanetSpec::new(200_000.0, 14, DEFAULT_GRID_SEGMENTS)?,
            ElevationRange::new(-2_000.0, 2_000.0)?,
            1e-4,
        ),
    ];

    let aspect = 16.0 / 9.0;
    let height_px = 1080.0;
    let ndc_x = 0.22;
    let ndc_y = -0.14;

    for (planet, range, hit_tol_m) in cases {
        let r = planet.radius_meters();
        let sampler = RidgeSampler { range };
        let mut cam = GlobeCameraState::new(planet, EcefMeters::from_raw(0.4, 0.3, 0.85))?;
        cam.altitude_meters = r * 1.2;
        cam.sync_terrain_elevations(&sampler);

        let view0 = cam.to_camera_view_f64(aspect, height_px)?;
        let Some((world_unit, cos_inc)) = cam.ray_surface_hit(&view0, ndc_x, ndc_y, &sampler)
        else {
            return Err(GlobeError::InvalidCameraProjection);
        };
        assert!(cos_inc > 0.0);

        // Verify ray actually passes through the displaced surface radius within hit_tol_m.
        let half_tan_y = (view0.fov_y_rad * 0.5).tan();
        let half_tan_x = half_tan_y * view0.aspect;
        let ray_dir = (view0.forward
            + view0.right.scale(ndc_x * half_tan_x)
            + view0.up.scale(ndc_y * half_tan_y))
        .normalized()?;
        let hit_surface = world_unit.scale(r + sampler.sample_elevation(world_unit));
        let along = (hit_surface - view0.position).dot(ray_dir);
        let ray_closest = view0.position + ray_dir.scale(along);
        assert!(ray_closest.distance_to(hit_surface) <= hit_tol_m);

        cam.zoom_anchor = Some(ZoomCursorAnchor {
            world_unit,
            ndc_x,
            ndc_y,
        });
        for step in 1..=20_u32 {
            let t = f64::from(step) / 20.0;
            cam.altitude_meters = (r * 1.2).powf(1.0 - t) * MIN_ALTITUDE_METERS.powf(t);
            cam.apply_zoom_to_cursor_with_elevation(aspect, height_px, &sampler);

            let view_now = cam.to_camera_view_f64(aspect, height_px)?;
            let Some((rehit, _)) = cam.ray_surface_hit(&view_now, ndc_x, ndc_y, &sampler) else {
                return Err(GlobeError::InvalidCameraProjection);
            };
            let err_m = rehit.distance_to(world_unit) * r;
            assert!(
                err_m <= 1e-3,
                "terrain zoom-to-cursor drifted {err_m:.3e} m at step {step}"
            );
        }

        let (near, far) = dynamic_clip_planes(MIN_ALTITUDE_METERS, r);
        assert!(near > 0.0 && far > near && f64::from(far) >= r * 2.5);
    }
    Ok(())
}

#[test]
fn terrain_elevation_resource_bumps_revision_on_sampler_change() -> Result<(), GlobeError> {
    let mut terrain = TerrainElevation::default();
    assert_eq!(terrain.revision(), 0);
    assert!(terrain.sampler().global_range().is_flat());

    let ridge = Arc::new(RidgeSampler {
        range: ElevationRange::new(-2_000.0, 2_000.0)?,
    });
    terrain.set_sampler(ridge);
    assert_eq!(terrain.revision(), 1);
    assert!(!terrain.sampler().global_range().is_flat());
    Ok(())
}
