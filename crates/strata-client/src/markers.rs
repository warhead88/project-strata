use std::collections::HashMap;
use std::fmt::Write as _;

use bevy::prelude::*;
use strata_globe::{ControlPointKind, PointId, ecef_to_tile_uv};

use crate::camera::{GlobeCameraState, OriginState};
use crate::globe_render::GlobeRenderState;
use crate::snapshot::{ClientWorldView, SelectedPointState};
use crate::terrain::{TerrainElevation, sample_clamped_elevation};

/// Marker entity tracking authoritative simulation points in render space.
#[derive(Resource, Default)]
pub struct MarkerRenderMap {
    entities: HashMap<PointId, Entity>,
    marker_mesh: Option<Handle<Mesh>>,
    active_material: Option<Handle<StandardMaterial>>,
    inactive_material: Option<Handle<StandardMaterial>>,
}

/// Tag component for the diagnostic HUD text node.
#[derive(Component)]
pub struct GlobeHudText;

/// Spawns marker assets and the diagnostic HUD overlay.
pub fn setup_markers_and_hud(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    let sphere_mesh = meshes.add(Sphere::new(1.0).mesh().uv(16, 12));
    let active_mat = materials.add(StandardMaterial {
        base_color: Color::srgb(1.0, 0.45, 0.12),
        emissive: LinearRgba::rgb(0.9, 0.35, 0.05),
        ..default()
    });
    let inactive_mat = materials.add(StandardMaterial {
        base_color: Color::srgb(0.25, 0.85, 0.95),
        emissive: LinearRgba::rgb(0.1, 0.4, 0.5),
        ..default()
    });

    commands.insert_resource(MarkerRenderMap {
        entities: HashMap::new(),
        marker_mesh: Some(sphere_mesh),
        active_material: Some(active_mat),
        inactive_material: Some(inactive_mat),
    });

    commands
        .spawn((
            Node {
                position_type: PositionType::Absolute,
                top: Val::Px(12.0),
                left: Val::Px(12.0),
                padding: UiRect::all(Val::Px(10.0)),
                ..default()
            },
            BackgroundColor(Color::srgba(0.04, 0.06, 0.09, 0.78)),
        ))
        .with_children(|parent| {
            parent.spawn((
                Text::new("Project Strata — Globe Initializing"),
                TextFont {
                    font_size: FontSize::Px(13.5),
                    ..default()
                },
                TextColor(Color::srgb(0.92, 0.95, 0.98)),
                GlobeHudText,
            ));
        });
}

/// Projects authoritative snapshot points through [`OriginState`] without mutating
/// simulation coordinates, draws local tangent gizmos, and updates the HUD.
#[expect(
    clippy::too_many_arguments,
    clippy::needless_pass_by_value,
    reason = "Bevy presentation system requires Res<T> by value across snapshot, terrain, selection, camera, origin, render stats, and gizmos"
)]
pub fn sync_markers_and_hud(
    mut commands: Commands,
    mut gizmos: Gizmos,
    world_view: Res<ClientWorldView>,
    terrain: Res<TerrainElevation>,
    selected: Res<SelectedPointState>,
    cam_state: Res<GlobeCameraState>,
    origin_state: Res<OriginState>,
    render_state: Res<GlobeRenderState>,
    mut marker_map: ResMut<MarkerRenderMap>,
    mut marker_query: Query<(&mut Transform, &mut MeshMaterial3d<StandardMaterial>)>,
    mut hud_query: Query<&mut Text, With<GlobeHudText>>,
) {
    let snapshot = world_view.snapshot();
    let origin = origin_state.origin;
    let (eye_ecef, _, _) = cam_state.compute_pose_f64();
    let sampler = terrain.sampler();
    let is_flat = sampler.global_range().is_flat();
    let r_base = snapshot.planet.radius_meters();
    let min_r = 1.0_f64.min(r_base * 0.5);

    let Some(mesh_handle) = marker_map.marker_mesh.clone() else {
        return;
    };
    let Some(active_mat) = marker_map.active_material.clone() else {
        return;
    };
    let Some(inactive_mat) = marker_map.inactive_material.clone() else {
        return;
    };

    let active_ids: std::collections::HashSet<PointId> =
        snapshot.points.iter().map(|p| p.id).collect();
    let stale_ids: Vec<PointId> = marker_map
        .entities
        .keys()
        .copied()
        .filter(|id| !active_ids.contains(id))
        .collect();
    for id in stale_ids {
        if let Some(entity) = marker_map.entities.remove(&id) {
            commands.entity(entity).despawn();
        }
    }

    for pt in &snapshot.points {
        let is_selected = pt.id == selected.selected_id;
        let display_pos = if !is_flat
            && pt.kind != ControlPointKind::WorldEntity
            && pt.elevation_m == 0.0
            && let Ok(unit) = pt.position.normalized()
        {
            let ground_m = sample_clamped_elevation(snapshot.planet, unit, sampler);
            unit.scale((r_base + ground_m).max(min_r))
        } else {
            pt.position
        };
        let rel = origin.to_render_vec3(display_pos);
        let dist = eye_ecef.distance_to(display_pos);
        #[expect(
            clippy::cast_possible_truncation,
            reason = "marker billboard scale is purely a visual f32 transform scale"
        )]
        let radius_scale = (dist * 0.0085).clamp(0.35, 160_000.0) as f32;
        let target_mat = if is_selected {
            active_mat.clone()
        } else {
            inactive_mat.clone()
        };

        if let Some(&entity) = marker_map.entities.get(&pt.id) {
            if let Ok((mut tf, mut mat)) = marker_query.get_mut(entity) {
                tf.translation = Vec3::new(rel.x, rel.y, rel.z);
                tf.scale = Vec3::splat(radius_scale);
                mat.0 = target_mat;
            }
        } else {
            let entity = commands
                .spawn((
                    Mesh3d(mesh_handle.clone()),
                    MeshMaterial3d(target_mat),
                    Transform::from_xyz(rel.x, rel.y, rel.z).with_scale(Vec3::splat(radius_scale)),
                ))
                .id();
            marker_map.entities.insert(pt.id, entity);
        }

        if is_selected {
            draw_selected_crosshair(&mut gizmos, origin, display_pos, dist);
        }
    }

    if let Ok(mut hud_text) = hud_query.single_mut() {
        hud_text.0 = format_hud(
            snapshot,
            selected.active_point(snapshot),
            &cam_state,
            &origin_state,
            &render_state,
            selected.auto_sweep,
        );
    }
}

fn draw_selected_crosshair(
    gizmos: &mut Gizmos,
    origin: strata_globe::LocalOrigin,
    point: strata_globe::EcefMeters,
    camera_dist: f64,
) {
    let Ok(normal) = point.normalized() else {
        return;
    };
    let ref_axis = if normal.y.abs() < 0.9 {
        strata_globe::EcefMeters::from_raw(0.0, 1.0, 0.0)
    } else {
        strata_globe::EcefMeters::from_raw(0.0, 0.0, 1.0)
    };
    let Ok(east) = ref_axis.cross(normal).normalized() else {
        return;
    };
    let Ok(north) = normal.cross(east).normalized() else {
        return;
    };

    let span = (camera_dist * 0.035).clamp(1.2, 450_000.0);
    let to_v3 = |p: strata_globe::EcefMeters| -> Vec3 {
        let r = origin.to_render_vec3(p);
        Vec3::new(r.x, r.y, r.z)
    };

    let center = to_v3(point);
    gizmos.line(
        center,
        to_v3(point + normal.scale(span * 1.5)),
        Color::srgb(1.0, 0.85, 0.2),
    );
    gizmos.line(
        to_v3(point - east.scale(span)),
        to_v3(point + east.scale(span)),
        Color::srgb(1.0, 0.45, 0.2),
    );
    gizmos.line(
        to_v3(point - north.scale(span)),
        to_v3(point + north.scale(span)),
        Color::srgb(0.35, 0.95, 0.45),
    );
}

fn format_hud(
    snapshot: &strata_globe::GlobeSnapshot,
    active: Option<&strata_globe::AuthoritativePoint>,
    cam_state: &GlobeCameraState,
    origin_state: &OriginState,
    render_state: &GlobeRenderState,
    auto_sweep: bool,
) -> String {
    let mut out = String::with_capacity(512);
    let _ = writeln!(
        out,
        "Project Strata | Planet R = {:.0} km | Tick {} | Tiles {} (LOD {}..{})",
        snapshot.planet.radius_meters() / 1_000.0,
        snapshot.tick,
        render_state.active_tile_count,
        render_state.min_lod,
        render_state.max_lod,
    );
    let blend = cam_state.orbital_blend();
    let view_mode = if blend >= 0.999 {
        "Planet (Center)"
    } else if blend <= 0.001 {
        "Surface"
    } else {
        "Transition"
    };
    let _ = writeln!(
        out,
        "Altitude: {:.1} m | View: {view_mode} (pitch {:.1} deg) | Rebases: {} | Sweep: {}",
        cam_state.altitude_meters,
        cam_state.effective_pitch_rad().to_degrees(),
        origin_state.rebase_count,
        if auto_sweep { "ON" } else { "OFF" },
    );
    if let Some(pt) = active {
        let tile_info = ecef_to_tile_uv(snapshot.planet, pt.position, render_state.max_lod)
            .map_or_else(
                |_| String::from("n/a"),
                |(tile, _, _)| {
                    format!(
                        "{} L{} ({}, {})",
                        tile.face.label(),
                        tile.lod,
                        tile.x,
                        tile.y
                    )
                },
            );
        let rel = origin_state.origin.to_render_vec3(pt.position);
        let _ = writeln!(
            out,
            "Selected [{}]: {} | Tile {}",
            pt.id.0, pt.label, tile_info
        );
        let _ = writeln!(
            out,
            "  ECEF f64: ({:.3}, {:.3}, {:.3}) m",
            pt.position.x, pt.position.y, pt.position.z
        );
        let _ = writeln!(
            out,
            "  Render f32: ({:.3}, {:.3}, {:.3}) m",
            rel.x, rel.y, rel.z
        );
    }
    let _ = write!(
        out,
        "Controls: Wheel/+-/PgUpDn Zoom-to-Cursor | MMB/WASD/Arrows Pan | RMB/Alt+MMB/Q-E Rotate+Tilt | N/Home/Compass North | 1-7/Tab Point | P/L/R"
    );
    out
}
