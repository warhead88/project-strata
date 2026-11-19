use std::f64::consts::PI;

use strata_globe::{EcefMeters, GlobeError, PlanetSpec};

use super::{
    GlobeCameraState, MIN_ALTITUDE_METERS, NORTH_RESET_DURATION_SECS, OriginState,
    ZoomCursorAnchor, dynamic_clip_planes,
};

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
