use bevy::input::mouse::{MouseMotion, MouseScrollUnit, MouseWheel};
use bevy::prelude::*;

use crate::camera::{GlobeCameraState, MIN_ALTITUDE_METERS, ZoomCursorAnchor};
use crate::snapshot::{ClientWorldView, SelectedPointState};
use crate::terrain::TerrainElevation;

/// Minimum cursor travel in pixels before RMB drag starts rotating/tilting the camera,
/// preserving crisp RMB clicks for issuing strategy orders.
pub const RMB_DRAG_THRESHOLD_PX: f32 = 5.0;

/// Tracks pointer gesture state (RMB click-vs-drag threshold and MMB ray-grab pan).
#[derive(Resource, Debug, Default, Clone)]
pub struct PointerGestureState {
    rmb_accumulated: Vec2,
    rmb_orbit_active: bool,
    prev_mmb_cursor: Option<Vec2>,
    /// Cursor position of the most recent RMB click below [`RMB_DRAG_THRESHOLD_PX`] (reserved for orders).
    pub last_right_click_cursor: Option<Vec2>,
}

impl PointerGestureState {
    /// Updates RMB drag state and returns `true` only once cursor travel exceeds [`RMB_DRAG_THRESHOLD_PX`].
    pub fn update_rmb_drag(&mut self, just_pressed: bool, pressed: bool, delta: Vec2) -> bool {
        if just_pressed {
            self.rmb_accumulated = Vec2::ZERO;
            self.rmb_orbit_active = false;
        }
        if !pressed {
            self.rmb_accumulated = Vec2::ZERO;
            self.rmb_orbit_active = false;
            return false;
        }
        if !self.rmb_orbit_active {
            self.rmb_accumulated += delta;
            if self.rmb_accumulated.length() >= RMB_DRAG_THRESHOLD_PX {
                self.rmb_orbit_active = true;
            }
        }
        self.rmb_orbit_active
    }

    /// Classifies an RMB release: returns `true` when it was a clean order click rather than an orbit drag.
    pub fn finish_rmb_click(&mut self, cursor_pos: Option<Vec2>) -> bool {
        let was_click = !self.rmb_orbit_active;
        if was_click {
            self.last_right_click_cursor = cursor_pos;
        }
        self.rmb_accumulated = Vec2::ZERO;
        self.rmb_orbit_active = false;
        was_click
    }
}

/// Processes strategy-map mouse and keyboard controls:
/// - Zoom: Wheel (Zoom-to-Cursor), `+/-`, `PageUp/PageDown`
/// - Pan: `MMB + Drag`, `WASD`, `Arrows` (LMB stays free for selection)
/// - Rotate & Tilt: `RMB + Drag` (5 px threshold), `Alt + MMB + Drag`, `Q/E`, `Alt + Up/Down`
/// - Reset to North (0.3 s): `N`, `Home`, or Compass UI button
#[expect(
    clippy::too_many_arguments,
    clippy::needless_pass_by_value,
    reason = "Bevy ECS system parameters require Res<T> by value across input and window resources"
)]
pub fn handle_camera_input(
    time: Res<Time>,
    windows: Query<&Window>,
    keys: Res<ButtonInput<KeyCode>>,
    mouse_buttons: Res<ButtonInput<MouseButton>>,
    mut wheel_events: MessageReader<MouseWheel>,
    mut motion_events: MessageReader<MouseMotion>,
    world_view: Res<ClientWorldView>,
    terrain: Res<TerrainElevation>,
    mut gesture: ResMut<PointerGestureState>,
    mut selected: ResMut<SelectedPointState>,
    mut cam_state: ResMut<GlobeCameraState>,
) {
    let dt = f64::from(time.delta_secs()).min(0.1);
    let (width_px, height_px, cursor_pos) =
        windows
            .iter()
            .next()
            .map_or((1280.0_f64, 720.0_f64, None), |w| {
                (
                    f64::from(w.width()).max(1.0),
                    f64::from(w.height()).max(1.0),
                    w.cursor_position(),
                )
            });

    apply_zoom_and_shortcuts(
        dt,
        width_px,
        height_px,
        cursor_pos,
        &keys,
        &mut wheel_events,
        &world_view,
        &terrain,
        &mut selected,
        &mut cam_state,
    );
    apply_pan_and_rotation(
        dt,
        width_px,
        height_px,
        cursor_pos,
        &keys,
        &mouse_buttons,
        &mut motion_events,
        &terrain,
        &mut gesture,
        &mut cam_state,
    );
}

#[expect(
    clippy::too_many_arguments,
    reason = "input helper bundles viewport, keyboard, wheel, snapshot, terrain, and camera state"
)]
fn apply_zoom_and_shortcuts(
    dt: f64,
    width_px: f64,
    height_px: f64,
    cursor_pos: Option<Vec2>,
    keys: &ButtonInput<KeyCode>,
    wheel_events: &mut MessageReader<MouseWheel>,
    world_view: &ClientWorldView,
    terrain: &TerrainElevation,
    selected: &mut SelectedPointState,
    cam_state: &mut GlobeCameraState,
) {
    let snapshot = world_view.snapshot();
    let max_alt = cam_state.max_altitude_meters();
    let aspect = width_px / height_px;

    let mut wheel_delta = 0.0_f64;
    for ev in wheel_events.read() {
        let step = match ev.unit {
            MouseScrollUnit::Line => f64::from(ev.y) * 0.18,
            MouseScrollUnit::Pixel => f64::from(ev.y) * 0.008,
        };
        wheel_delta += step;
    }
    wheel_delta = wheel_delta.clamp(-10.0, 10.0);
    if wheel_delta != 0.0 {
        selected.auto_sweep = false;
        if let Some(cursor) = cursor_pos
            && let Ok(cam_view) = cam_state.to_camera_view_f64(aspect, height_px)
        {
            let (ndc_x, ndc_y) = cursor_to_ndc(cursor, width_px, height_px);
            cam_state.zoom_anchor = cam_state
                .ray_surface_hit(&cam_view, ndc_x, ndc_y, terrain.sampler())
                .map(|(world_unit, _)| ZoomCursorAnchor {
                    world_unit,
                    ndc_x,
                    ndc_y,
                });
        } else {
            cam_state.zoom_anchor = None;
        }
        let factor = (-wheel_delta).exp();
        cam_state.target_altitude_meters =
            (cam_state.target_altitude_meters * factor).clamp(MIN_ALTITUDE_METERS, max_alt);
    }

    let mut zoom_dir = 0.0_f64;
    if keys.pressed(KeyCode::Equal)
        || keys.pressed(KeyCode::NumpadAdd)
        || keys.pressed(KeyCode::PageUp)
    {
        zoom_dir -= 1.0;
    }
    if keys.pressed(KeyCode::Minus)
        || keys.pressed(KeyCode::NumpadSubtract)
        || keys.pressed(KeyCode::PageDown)
    {
        zoom_dir += 1.0;
    }
    if zoom_dir != 0.0 {
        selected.auto_sweep = false;
        cam_state.zoom_anchor = None;
        cam_state.target_altitude_meters = (cam_state.target_altitude_meters
            * (zoom_dir * 1.4 * dt).exp())
        .clamp(MIN_ALTITUDE_METERS, max_alt);
    }

    if keys.just_pressed(KeyCode::KeyN) || keys.just_pressed(KeyCode::Home) {
        cam_state.start_north_reset();
    }

    if keys.just_pressed(KeyCode::KeyP) {
        selected.auto_sweep = false;
        cam_state.zoom_anchor = None;
        cam_state.target_altitude_meters = (cam_state.planet.radius_meters() * 2.8).min(max_alt);
    }
    if keys.just_pressed(KeyCode::KeyL) {
        selected.auto_sweep = false;
        if let Some(pt) = selected.active_point(snapshot) {
            cam_state.focus_on_point(pt.position, Some(MIN_ALTITUDE_METERS));
            cam_state.sync_terrain_elevations(terrain.sampler());
        } else {
            cam_state.zoom_anchor = None;
            cam_state.target_altitude_meters = MIN_ALTITUDE_METERS;
        }
    }
    if keys.just_pressed(KeyCode::KeyR) {
        selected.auto_sweep = !selected.auto_sweep;
        cam_state.zoom_anchor = None;
    }

    let mut point_changed = false;
    if keys.just_pressed(KeyCode::Tab) {
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
        cam_state.sync_terrain_elevations(terrain.sampler());
    }
}

#[expect(
    clippy::too_many_arguments,
    reason = "input helper combines viewport, buttons, motion events, terrain, gesture state, and camera state"
)]
fn apply_pan_and_rotation(
    dt: f64,
    width_px: f64,
    height_px: f64,
    cursor_pos: Option<Vec2>,
    keys: &ButtonInput<KeyCode>,
    mouse_buttons: &ButtonInput<MouseButton>,
    motion_events: &mut MessageReader<MouseMotion>,
    terrain: &TerrainElevation,
    gesture: &mut PointerGestureState,
    cam_state: &mut GlobeCameraState,
) {
    let mut drag_delta = Vec2::ZERO;
    for ev in motion_events.read() {
        drag_delta += ev.delta;
    }

    let alt_held = keys.pressed(KeyCode::AltLeft) || keys.pressed(KeyCode::AltRight);
    if mouse_buttons.just_released(MouseButton::Right) {
        let _ = gesture.finish_rmb_click(cursor_pos);
    }
    let rmb_orbit = gesture.update_rmb_drag(
        mouse_buttons.just_pressed(MouseButton::Right),
        mouse_buttons.pressed(MouseButton::Right),
        drag_delta,
    );

    let alt_mmb_orbit = alt_held && mouse_buttons.pressed(MouseButton::Middle);
    if (rmb_orbit || alt_mmb_orbit) && drag_delta != Vec2::ZERO {
        cam_state.zoom_anchor = None;
        cam_state.rotate_yaw(f64::from(drag_delta.x) * 0.005);
        cam_state.adjust_pitch(f64::from(drag_delta.y) * 0.005);
    }

    let mmb_pan = !alt_held && mouse_buttons.pressed(MouseButton::Middle);
    if mmb_pan {
        let mut panned_via_ray = false;
        if let (Some(prev_c), Some(curr_c)) = (gesture.prev_mmb_cursor, cursor_pos)
            && prev_c != curr_c
            && let Ok(cam_view) = cam_state.to_camera_view_f64(width_px / height_px, height_px)
        {
            let prev_ndc = cursor_to_ndc(prev_c, width_px, height_px);
            let curr_ndc = cursor_to_ndc(curr_c, width_px, height_px);
            let sampler = terrain.sampler();
            if let (Some((prev_unit, prev_cos)), Some((curr_unit, curr_cos))) = (
                cam_state.ray_surface_hit(&cam_view, prev_ndc.0, prev_ndc.1, sampler),
                cam_state.ray_surface_hit(&cam_view, curr_ndc.0, curr_ndc.1, sampler),
            ) && prev_cos > 0.08
                && curr_cos > 0.08
            {
                cam_state.zoom_anchor = None;
                panned_via_ray = cam_state.rotate_sphere_from_to(curr_unit, prev_unit, 0.35);
            }
        }
        if !panned_via_ray && drag_delta != Vec2::ZERO {
            cam_state.zoom_anchor = None;
            let ang_scale =
                (cam_state.altitude_meters / cam_state.planet.radius_meters()).clamp(1e-7, 0.5);
            cam_state.pan_tangent(
                -f64::from(drag_delta.x) * 0.0025 * ang_scale,
                f64::from(drag_delta.y) * 0.0025 * ang_scale,
            );
        }
        cam_state.sync_terrain_elevations(terrain.sampler());
        gesture.prev_mmb_cursor = cursor_pos;
    } else {
        gesture.prev_mmb_cursor = None;
    }

    let mut pan_right = 0.0_f64;
    let mut pan_fwd = 0.0_f64;
    if keys.pressed(KeyCode::KeyA) || (!alt_held && keys.pressed(KeyCode::ArrowLeft)) {
        pan_right -= 1.0;
    }
    if keys.pressed(KeyCode::KeyD) || (!alt_held && keys.pressed(KeyCode::ArrowRight)) {
        pan_right += 1.0;
    }
    if keys.pressed(KeyCode::KeyW) || (!alt_held && keys.pressed(KeyCode::ArrowUp)) {
        pan_fwd += 1.0;
    }
    if keys.pressed(KeyCode::KeyS) || (!alt_held && keys.pressed(KeyCode::ArrowDown)) {
        pan_fwd -= 1.0;
    }
    if pan_right != 0.0 || pan_fwd != 0.0 {
        cam_state.zoom_anchor = None;
        let ang_speed =
            (cam_state.altitude_meters / cam_state.planet.radius_meters()).clamp(2e-7, 0.8);
        cam_state.pan_tangent(pan_right * ang_speed * dt, pan_fwd * ang_speed * dt);
        cam_state.sync_terrain_elevations(terrain.sampler());
    }

    let mut yaw_dir = 0.0_f64;
    if keys.pressed(KeyCode::KeyQ) {
        yaw_dir -= 1.0;
    }
    if keys.pressed(KeyCode::KeyE) {
        yaw_dir += 1.0;
    }
    if yaw_dir != 0.0 {
        cam_state.zoom_anchor = None;
        cam_state.rotate_yaw(yaw_dir * 1.4 * dt);
    }
    if alt_held && keys.pressed(KeyCode::ArrowUp) {
        cam_state.adjust_pitch(0.9 * dt);
    }
    if alt_held && keys.pressed(KeyCode::ArrowDown) {
        cam_state.adjust_pitch(-0.9 * dt);
    }
}

fn cursor_to_ndc(cursor: Vec2, width_px: f64, height_px: f64) -> (f64, f64) {
    let ndc_x = (f64::from(cursor.x) / width_px) * 2.0 - 1.0;
    let ndc_y = 1.0 - (f64::from(cursor.y) / height_px) * 2.0;
    (ndc_x, ndc_y)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rmb_click_below_threshold_does_not_trigger_orbit() {
        let mut gesture = PointerGestureState::default();
        assert!(!gesture.update_rmb_drag(true, true, Vec2::new(2.0, 1.5)));
        assert!(!gesture.update_rmb_drag(false, true, Vec2::new(1.0, 1.0)));
        assert!(gesture.finish_rmb_click(Some(Vec2::new(400.0, 300.0))));
        assert_eq!(
            gesture.last_right_click_cursor,
            Some(Vec2::new(400.0, 300.0))
        );
    }

    #[test]
    fn rmb_drag_above_threshold_engages_orbit_and_suppresses_click() {
        let mut gesture = PointerGestureState::default();
        assert!(!gesture.update_rmb_drag(true, true, Vec2::new(2.0, 0.0)));
        assert!(gesture.update_rmb_drag(false, true, Vec2::new(4.0, 0.0)));
        assert!(!gesture.finish_rmb_click(Some(Vec2::new(410.0, 300.0))));
        assert_eq!(gesture.last_right_click_cursor, None);
    }
}
