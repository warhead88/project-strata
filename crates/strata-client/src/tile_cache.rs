use std::collections::{HashMap, HashSet};

use bevy::prelude::*;
use strata_globe::{CoarserEdges, LocalOrigin, TileId};

use crate::tile_mesh_build::{BuiltTileMeshes, to_bevy_mesh};

/// Cached GPU tile entities and metadata for a single [`TileId`].
pub(crate) struct CachedTile {
    pub terrain_entity: Entity,
    pub terrain_mesh: Handle<Mesh>,
    pub water_entity: Option<Entity>,
    pub water_mesh: Option<Handle<Mesh>>,
    pub coarser_edges: CoarserEdges,
    pub tile_origin: LocalOrigin,
    pub last_used_frame: u64,
}

pub(crate) type TileEntityQuery<'w, 's> = Query<
    'w,
    's,
    (
        &'static mut Transform,
        &'static mut Visibility,
        &'static mut MeshMaterial3d<StandardMaterial>,
    ),
>;

#[expect(
    clippy::too_many_arguments,
    reason = "GPU mesh upload updates tile assets, entity transforms, and material handles"
)]
pub(crate) fn upload_built_tile(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    tiles: &mut HashMap<TileId, CachedTile>,
    tile_query: &mut TileEntityQuery<'_, '_>,
    origin: LocalOrigin,
    frame: u64,
    terrain_mat: Handle<StandardMaterial>,
    water_mat: Handle<StandardMaterial>,
    built: BuiltTileMeshes,
) {
    let rel = origin.to_render_vec3(built.tile_origin.ecef);
    let translation = Vec3::new(rel.x, rel.y, rel.z);

    if let Some(existing) = tiles.get_mut(&built.tile) {
        if let Some(mut mesh_asset) = meshes.get_mut(&existing.terrain_mesh) {
            *mesh_asset = to_bevy_mesh(built.terrain_mesh);
        }
        if let Ok((mut tf, _, _)) = tile_query.get_mut(existing.terrain_entity) {
            tf.translation = translation;
        }
        match (
            existing.water_entity,
            existing.water_mesh.as_ref(),
            built.water_mesh,
        ) {
            (Some(w_entity), Some(w_handle), Some(new_water)) => {
                if let Some(mut w_asset) = meshes.get_mut(w_handle) {
                    *w_asset = to_bevy_mesh(new_water);
                }
                if let Ok((mut tf, _, _)) = tile_query.get_mut(w_entity) {
                    tf.translation = translation;
                }
            }
            (None, None, Some(new_water)) => {
                let w_handle = meshes.add(to_bevy_mesh(new_water));
                let w_entity = commands
                    .spawn((
                        Mesh3d(w_handle.clone()),
                        MeshMaterial3d(water_mat),
                        Transform::from_translation(translation),
                        Visibility::Hidden,
                    ))
                    .id();
                existing.water_entity = Some(w_entity);
                existing.water_mesh = Some(w_handle);
            }
            (Some(w_entity), Some(_), None) => {
                commands.entity(w_entity).despawn();
                if let Some(w_handle) = existing.water_mesh.take() {
                    meshes.remove(&w_handle);
                }
                existing.water_entity = None;
            }
            _ => {}
        }
        existing.coarser_edges = built.coarser_edges;
        existing.tile_origin = built.tile_origin;
        existing.last_used_frame = frame;
        return;
    }

    let terrain_mesh = meshes.add(to_bevy_mesh(built.terrain_mesh));
    let terrain_entity = commands
        .spawn((
            Mesh3d(terrain_mesh.clone()),
            MeshMaterial3d(terrain_mat),
            Transform::from_translation(translation),
            Visibility::Hidden,
        ))
        .id();
    let (water_entity, water_mesh) = match built.water_mesh {
        Some(w_data) => {
            let w_handle = meshes.add(to_bevy_mesh(w_data));
            let w_entity = commands
                .spawn((
                    Mesh3d(w_handle.clone()),
                    MeshMaterial3d(water_mat),
                    Transform::from_translation(translation),
                    Visibility::Hidden,
                ))
                .id();
            (Some(w_entity), Some(w_handle))
        }
        None => (None, None),
    };
    tiles.insert(
        built.tile,
        CachedTile {
            terrain_entity,
            terrain_mesh,
            water_entity,
            water_mesh,
            coarser_edges: built.coarser_edges,
            tile_origin: built.tile_origin,
            last_used_frame: frame,
        },
    );
}

#[derive(Clone, Copy)]
pub(crate) struct TileSyncFrame {
    pub origin: LocalOrigin,
    pub origin_changed: bool,
    pub overlay_changed: bool,
}

pub(crate) fn sync_tile_entities(
    commands: &mut Commands,
    tiles: &HashMap<TileId, CachedTile>,
    tile_query: &mut TileEntityQuery<'_, '_>,
    frame: TileSyncFrame,
    visible: &HashSet<TileId>,
    material_for: impl Fn(TileId) -> Handle<StandardMaterial>,
) {
    let origin = frame.origin;
    let origin_changed = frame.origin_changed;
    let overlay_changed = frame.overlay_changed;
    for (&tile_id, cached) in tiles {
        let target_vis = if visible.contains(&tile_id) {
            Visibility::Inherited
        } else {
            Visibility::Hidden
        };
        let rel = origin.to_render_vec3(cached.tile_origin.ecef);
        let target_pos = Vec3::new(rel.x, rel.y, rel.z);

        if let Ok((mut tf, mut vis, mut mat)) = tile_query.get_mut(cached.terrain_entity) {
            if origin_changed {
                tf.translation = target_pos;
            }
            if *vis != target_vis {
                *vis = target_vis;
            }
            if overlay_changed {
                mat.0 = material_for(tile_id);
            }
        } else if target_vis != Visibility::Hidden {
            commands.entity(cached.terrain_entity).insert(target_vis);
        }

        if let Some(w_entity) = cached.water_entity {
            if let Ok((mut tf, mut vis, _)) = tile_query.get_mut(w_entity) {
                if origin_changed {
                    tf.translation = target_pos;
                }
                if *vis != target_vis {
                    *vis = target_vis;
                }
            } else if target_vis != Visibility::Hidden {
                commands.entity(w_entity).insert(target_vis);
            }
        }
    }
}

pub(crate) fn despawn_cached_tile(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    tile: CachedTile,
) {
    commands.entity(tile.terrain_entity).despawn();
    meshes.remove(&tile.terrain_mesh);
    if let Some(w_entity) = tile.water_entity {
        commands.entity(w_entity).despawn();
    }
    if let Some(w_mesh) = tile.water_mesh {
        meshes.remove(&w_mesh);
    }
}
