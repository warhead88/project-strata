use bevy::input::mouse::{MouseMotion, MouseWheel};
use bevy::prelude::*;

use crate::camera::{
    GlobeCamera, GlobeCameraState, MIN_ALTITUDE_METERS, OriginState, dynamic_clip_planes,
};
use crate::snapshot::{ClientWorldView, SelectedPointState};

/// Spawns the 3D globe camera and directional sunlight.
#[expect(
    clippy::needless_pass_by_value,
    reason = "Bevy ECS system parameters require Res<T> by value"
)]
pub fn setup_camera_and_lighting(
    mut commands: Commands,
    cam_state: Res<GlobeCameraState>,
    origin_state: Res<OriginState>,
) {
    let (eye, target, up) = cam_state.compute_pose_f64();
    let eye_rel = origin_state.origin.to_render_vec3(eye);
    let tgt_rel = origin_state.origin.to_render_vec3(target);
    let (near, far) =
        dynamic_clip_planes(cam_state.altitude_meters, cam_state.planet.radius_meters());

    #[expect(
        clippy::cast_possible_truncation,
        reason = "camera unit up vector narrows cleanly to Bevy f32 transforms"
    )]
    let transform = Transform::from_xyz(eye_rel.x, eye_rel.y, eye_rel.z).looking_at(
        Vec3::new(tgt_rel.x, tgt_rel.y, tgt_rel.z),
        Vec3::new(up.x as f32, up.y as f32, up.z as f32),
    );

    #[expect(
        clippy::cast_possible_truncation,
        reason = "vertical FOV narrows cleanly to f32"
    )]
    let fov = cam_state.fov_y_rad as f32;

    commands.spawn((
        Camera3d::default(),
        Projection::Perspective(PerspectiveProjection {
            fov,
            near,
            far,
            ..default()
        }),
        transform,
        GlobeCamera,
    ));

    commands.spawn((
        DirectionalLight {
            illuminance: 18_000.0,
            shadow_maps_enabled: false,
            ..default()
        },
        Transform::from_xyz(1.0, 0.8, 0.6).looking_at(Vec3::ZERO, Vec3::Y),
    ));

    commands.insert_resource(GlobalAmbientLight {
        color: Color::srgb(0.75, 0.82, 0.95),
        brightness: 420.0,
        affects_lightmapped_meshes: true,
    });
}

/// Processes keyboard and mouse controls for zoom, orbit, pan, and control-point cycling.
#[expect(
    clippy::too_many_arguments,
    clippy::needless_pass_by_value,
    reason = "Bevy ECS system parameters require Res<T> by value across input resources"
)]
pub fn handle_camera_input(
    time: Res<Time>,
    keys: Res<ButtonInput<KeyCode>>,
    mouse_buttons: Res<ButtonInput<MouseButton>>,
    mut wheel_events: MessageReader<MouseWheel>,
    mut motion_events: MessageReader<MouseMotion>,
    world_view: Res<ClientWorldView>,
    mut selected: ResMut<SelectedPointState>,
    mut cam_state: ResMut<GlobeCameraState>,
) {
    let dt = f64::from(time.delta_secs()).min(0.1);
    apply_zoom_and_point_selection(
        dt,
        &keys,
        &mut wheel_events,
        &world_view,
        &mut selected,
        &mut cam_state,
    );
    apply_orbit_and_pan(
        dt,
        &keys,
        &mouse_buttons,
        &mut motion_events,
        &mut cam_state,
    );
}

fn apply_zoom_and_point_selection(
    dt: f64,
    keys: &ButtonInput<KeyCode>,
    wheel_events: &mut MessageReader<MouseWheel>,
    world_view: &ClientWorldView,
    selected: &mut SelectedPointState,
    cam_state: &mut GlobeCameraState,
) {
    let snapshot = world_view.snapshot();
    let max_alt = cam_state.max_altitude_meters();

    for ev in wheel_events.read() {
        selected.auto_sweep = false;
        let factor = (-f64::from(ev.y) * 0.16).exp();
        cam_state.target_altitude_meters =
            (cam_state.target_altitude_meters * factor).clamp(MIN_ALTITUDE_METERS, max_alt);
    }

    let mut zoom_dir = 0.0_f64;
    if keys.pressed(KeyCode::Equal) || keys.pressed(KeyCode::NumpadAdd) {
        zoom_dir -= 1.0;
    }
    if keys.pressed(KeyCode::Minus) || keys.pressed(KeyCode::NumpadSubtract) {
        zoom_dir += 1.0;
    }
    if zoom_dir != 0.0 {
        selected.auto_sweep = false;
        cam_state.target_altitude_meters = (cam_state.target_altitude_meters
            * (zoom_dir * 1.4 * dt).exp())
        .clamp(MIN_ALTITUDE_METERS, max_alt);
    }

    if keys.just_pressed(KeyCode::KeyP) {
        selected.auto_sweep = false;
        cam_state.target_altitude_meters = (cam_state.planet.radius_meters() * 2.8).min(max_alt);
    }
    if keys.just_pressed(KeyCode::KeyL) {
        selected.auto_sweep = false;
        if let Some(pt) = selected.active_point(snapshot) {
            cam_state.focus_on_point(pt.position, Some(MIN_ALTITUDE_METERS));
        } else {
            cam_state.target_altitude_meters = MIN_ALTITUDE_METERS;
        }
    }
    if keys.just_pressed(KeyCode::KeyR) {
        selected.auto_sweep = !selected.auto_sweep;
    }

    let mut point_changed = false;
    if keys.just_pressed(KeyCode::Tab) || keys.just_pressed(KeyCode::KeyN) {
        selected.cycle(snapshot, 1);
        point_changed = true;
    }
    let digit_keys = [
        (KeyCode::Digit1, 1),
        (KeyCode::Digit2, 2),
        (KeyCode::Digit3, 3),
        (KeyCode::Digit4, 4),
        (KeyCode::Digit5, 5),
        (KeyCode::Digit6, 6),
        (KeyCode::Digit7, 7),
    ];
    for (code, num) in digit_keys {
        if keys.just_pressed(code) {
            selected.select_one_based(snapshot, num);
            point_changed = true;
        }
    }
    if point_changed && let Some(pt) = selected.active_point(snapshot) {
        cam_state.focus_on_point(pt.position, None);
    }
}

fn apply_orbit_and_pan(
    dt: f64,
    keys: &ButtonInput<KeyCode>,
    mouse_buttons: &ButtonInput<MouseButton>,
    motion_events: &mut MessageReader<MouseMotion>,
    cam_state: &mut GlobeCameraState,
) {
    let mut drag_delta = Vec2::ZERO;
    for ev in motion_events.read() {
        drag_delta += ev.delta;
    }
    if mouse_buttons.pressed(MouseButton::Left) && drag_delta != Vec2::ZERO {
        cam_state.yaw_rad += f64::from(drag_delta.x) * 0.005;
        cam_state.pitch_rad =
            (cam_state.pitch_rad + f64::from(drag_delta.y) * 0.005).clamp(0.0, 1.25);
    }
    if mouse_buttons.pressed(MouseButton::Right) && drag_delta != Vec2::ZERO {
        let ang_scale =
            (cam_state.altitude_meters / cam_state.planet.radius_meters()).clamp(1e-7, 0.5);
        cam_state.pan_tangent(
            -f64::from(drag_delta.x) * 0.0025 * ang_scale,
            f64::from(drag_delta.y) * 0.0025 * ang_scale,
        );
    }

    let mut pan_right = 0.0_f64;
    let mut pan_fwd = 0.0_f64;
    if keys.pressed(KeyCode::KeyA) {
        pan_right -= 1.0;
    }
    if keys.pressed(KeyCode::KeyD) {
        pan_right += 1.0;
    }
    if keys.pressed(KeyCode::KeyW) {
        pan_fwd += 1.0;
    }
    if keys.pressed(KeyCode::KeyS) {
        pan_fwd -= 1.0;
    }
    if pan_right != 0.0 || pan_fwd != 0.0 {
        let ang_speed =
            (cam_state.altitude_meters / cam_state.planet.radius_meters()).clamp(2e-7, 0.8);
        cam_state.pan_tangent(pan_right * ang_speed * dt, pan_fwd * ang_speed * dt);
    }

    if keys.pressed(KeyCode::ArrowLeft) {
        cam_state.yaw_rad -= 1.2 * dt;
    }
    if keys.pressed(KeyCode::ArrowRight) {
        cam_state.yaw_rad += 1.2 * dt;
    }
    if keys.pressed(KeyCode::ArrowUp) {
        cam_state.pitch_rad = (cam_state.pitch_rad + 0.9 * dt).clamp(0.0, 1.25);
    }
    if keys.pressed(KeyCode::ArrowDown) {
        cam_state.pitch_rad = (cam_state.pitch_rad - 0.9 * dt).clamp(0.0, 1.25);
    }
}

/// Updates logarithmic altitude damping, rebases [`OriginState`], and syncs Bevy [`Camera3d`].
#[expect(
    clippy::needless_pass_by_value,
    reason = "Bevy ECS system parameters require Res<T> by value"
)]
pub fn update_camera_and_origin(
    time: Res<Time>,
    world_view: Res<ClientWorldView>,
    mut selected: ResMut<SelectedPointState>,
    mut cam_state: ResMut<GlobeCameraState>,
    mut origin_state: ResMut<OriginState>,
    mut camera_query: Query<(&mut Transform, &mut Projection), With<GlobeCamera>>,
) {
    let dt = f64::from(time.delta_secs()).min(0.1);
    let snapshot_planet = world_view.snapshot().planet;
    if cam_state.planet != snapshot_planet {
        cam_state.planet = snapshot_planet;
        let max_alt = cam_state.max_altitude_meters();
        cam_state.altitude_meters = cam_state
            .altitude_meters
            .clamp(MIN_ALTITUDE_METERS, max_alt);
        cam_state.target_altitude_meters = cam_state
            .target_altitude_meters
            .clamp(MIN_ALTITUDE_METERS, max_alt);
    }
    let max_alt = cam_state.max_altitude_meters();

    if selected.auto_sweep {
        selected.sweep_phase = (selected.sweep_phase + dt * 0.22) % 2.0;
        let t = if selected.sweep_phase <= 1.0 {
            selected.sweep_phase
        } else {
            2.0 - selected.sweep_phase
        };
        let smooth_t = t * t * (3.0 - 2.0 * t);
        let orbital_alt = (cam_state.planet.radius_meters() * 3.1).min(max_alt);
        cam_state.target_altitude_meters =
            orbital_alt.powf(1.0 - smooth_t) * MIN_ALTITUDE_METERS.powf(smooth_t);
    }

    // Damping in log-altitude space feels uniform across 6 orders of magnitude.
    let blend = 1.0 - (-12.0 * dt).exp();
    let log_cur = cam_state.altitude_meters.ln();
    let log_tgt = cam_state.target_altitude_meters.ln();
    cam_state.altitude_meters = (log_cur + (log_tgt - log_cur) * blend).exp();

    let (eye, target, up) = cam_state.compute_pose_f64();
    if origin_state
        .policy
        .maybe_rebase(&mut origin_state.origin, eye, cam_state.altitude_meters)
    {
        origin_state.rebase_count += 1;
    }

    let eye_rel = origin_state.origin.to_render_vec3(eye);
    let tgt_rel = origin_state.origin.to_render_vec3(target);
    let (near, far) =
        dynamic_clip_planes(cam_state.altitude_meters, cam_state.planet.radius_meters());

    for (mut transform, mut projection) in &mut camera_query {
        #[expect(
            clippy::cast_possible_truncation,
            reason = "camera unit up vector narrows cleanly to f32"
        )]
        let up_f32 = Vec3::new(up.x as f32, up.y as f32, up.z as f32);
        *transform = Transform::from_xyz(eye_rel.x, eye_rel.y, eye_rel.z)
            .looking_at(Vec3::new(tgt_rel.x, tgt_rel.y, tgt_rel.z), up_f32);

        if let Projection::Perspective(persp) = &mut *projection {
            persp.near = near;
            persp.far = far;
        }
    }
}
