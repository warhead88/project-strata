use std::collections::{HashMap, HashSet};

use bevy::prelude::*;
use strata_globe::{ControlPointKind, PointId};

use crate::camera::{GlobeCameraState, OriginState};
use crate::globe_debug::TerrainDebug;
use crate::globe_render::GlobeRenderState;
use crate::hud::format_hud_into;
use crate::snapshot::{ClientWorldView, SelectedPointState};
use crate::terrain::{TerrainElevation, sample_clamped_elevation};
use crate::terrain_backend::TerrainBackend;

/// Marker entity tracking authoritative simulation points in render space.
#[derive(Resource, Default)]
pub struct MarkerRenderMap {
    entities: HashMap<PointId, Entity>,
    cached_point_ids: Vec<PointId>,
    active_ids_scratch: HashSet<PointId>,
    stale_ids_scratch: Vec<PointId>,
    hud_scratch: String,
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
        hud_scratch: String::with_capacity(640),
        marker_mesh: Some(sphere_mesh),
        active_material: Some(active_mat),
        inactive_material: Some(inactive_mat),
        ..default()
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
    render_state: Option<Res<GlobeRenderState>>,
    backend: Res<TerrainBackend>,
    debug: Res<TerrainDebug>,
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

    let marker_map = &mut *marker_map;
    reconcile_marker_ids(&mut commands, marker_map, &snapshot.points);

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
                let next_pos = Vec3::new(rel.x, rel.y, rel.z);
                if tf.translation != next_pos {
                    tf.translation = next_pos;
                }
                let next_scale = Vec3::splat(radius_scale);
                if tf.scale != next_scale {
                    tf.scale = next_scale;
                }
                if mat.0 != target_mat {
                    mat.0 = target_mat;
                }
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
        marker_map.hud_scratch.clear();
        format_hud_into(
            &mut marker_map.hud_scratch,
            debug.0,
            world_view.frame(),
            selected.active_point(snapshot),
            &cam_state,
            &origin_state,
            render_state.as_deref(),
            backend.uses_gpu(),
            terrain.world().map(std::sync::Arc::as_ref),
            selected.auto_sweep,
        );
        if hud_text.0 != marker_map.hud_scratch {
            hud_text.0.clone_from(&marker_map.hud_scratch);
        }
    }
}

fn reconcile_marker_ids(
    commands: &mut Commands,
    marker_map: &mut MarkerRenderMap,
    points: &[strata_globe::AuthoritativePoint],
) {
    let ids_unchanged = marker_map.cached_point_ids.len() == points.len()
        && marker_map
            .cached_point_ids
            .iter()
            .zip(points.iter())
            .all(|(&cached_id, pt)| cached_id == pt.id);
    if ids_unchanged {
        return;
    }
    marker_map.cached_point_ids.clear();
    marker_map
        .cached_point_ids
        .extend(points.iter().map(|p| p.id));
    marker_map.active_ids_scratch.clear();
    marker_map
        .active_ids_scratch
        .extend(marker_map.cached_point_ids.iter().copied());
    marker_map.stale_ids_scratch.clear();
    marker_map.stale_ids_scratch.extend(
        marker_map
            .entities
            .keys()
            .copied()
            .filter(|id| !marker_map.active_ids_scratch.contains(id)),
    );
    for id in marker_map.stale_ids_scratch.drain(..) {
        if let Some(entity) = marker_map.entities.remove(&id) {
            commands.entity(entity).despawn();
        }
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
