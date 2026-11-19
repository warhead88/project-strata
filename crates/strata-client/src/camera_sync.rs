use bevy::prelude::*;
use strata_globe::{EcefMeters, GlobeError, LocalOrigin, RebasePolicy};

use crate::camera::{GlobeCamera, GlobeCameraState, MIN_ALTITUDE_METERS};
use crate::snapshot::{ClientWorldView, SelectedPointState};

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

/// Rotates `v` around unit `axis` by `angle_rad` using Rodrigues' rotation formula.
#[must_use]
pub fn rotate_around_axis(v: EcefMeters, axis: EcefMeters, angle_rad: f64) -> EcefMeters {
    let (sin_a, cos_a) = angle_rad.sin_cos();
    v.scale(cos_a) + axis.cross(v).scale(sin_a) + axis.scale(axis.dot(v) * (1.0 - cos_a))
}

/// Computes an initial geographic North tangent at `normal`, falling back smoothly at the poles.
#[must_use]
pub fn initial_north_tangent(normal: EcefMeters) -> EcefMeters {
    let north = EcefMeters::from_raw(0.0, 1.0, 0.0);
    let proj = north - normal.scale(north.dot(normal));
    if proj.norm() > 1e-6
        && let Ok(unit) = proj.normalized()
    {
        return unit;
    }
    let fallback = if normal.y >= 0.0 {
        EcefMeters::from_raw(0.0, 0.0, -1.0)
    } else {
        EcefMeters::from_raw(0.0, 0.0, 1.0)
    };
    let fb_proj = fallback - normal.scale(fallback.dot(normal));
    fb_proj
        .normalized()
        .unwrap_or(EcefMeters::from_raw(0.0, 0.0, -1.0))
}

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

/// Updates logarithmic altitude damping, Zoom-to-Cursor alignment, North reset animation,
/// [`OriginState`] rebasing, and Bevy [`Camera3d`] transform & clip planes.
#[expect(
    clippy::needless_pass_by_value,
    reason = "Bevy ECS system parameters require Res<T> by value"
)]
pub fn update_camera_and_origin(
    time: Res<Time>,
    windows: Query<&Window>,
    world_view: Res<ClientWorldView>,
    mut selected: ResMut<SelectedPointState>,
    mut cam_state: ResMut<GlobeCameraState>,
    mut origin_state: ResMut<OriginState>,
    mut camera_query: Query<(&mut Transform, &mut Projection), With<GlobeCamera>>,
) {
    let dt = f64::from(time.delta_secs()).min(0.1);
    let (width_px, height_px) = windows.iter().next().map_or((1280.0_f64, 720.0_f64), |w| {
        (
            f64::from(w.width()).max(1.0),
            f64::from(w.height()).max(1.0),
        )
    });

    let snapshot_planet = world_view.snapshot().planet;
    if cam_state.planet != snapshot_planet {
        cam_state.planet = snapshot_planet;
        cam_state.zoom_anchor = None;
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

    let blend = 1.0 - (-12.0 * dt).exp();
    let log_cur = cam_state.altitude_meters.ln();
    let log_tgt = cam_state.target_altitude_meters.ln();
    cam_state.altitude_meters = (log_cur + (log_tgt - log_cur) * blend).exp();

    if cam_state.zoom_anchor.is_some() {
        cam_state.apply_zoom_to_cursor(width_px / height_px, height_px);
        if (cam_state.altitude_meters.ln() - log_tgt).abs() < 1e-4 {
            cam_state.zoom_anchor = None;
        }
    }
    cam_state.step_north_reset(dt);

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
