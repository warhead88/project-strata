use std::collections::{HashMap, HashSet};

use bevy::prelude::*;
use strata_globe::{
    CoarserEdges, LocalOrigin, LodConfig, PlanetSpec, TileId, select_tiles_with_elevation,
};

use crate::camera::{GlobeCameraState, OriginState};
use crate::globe_debug::build_tile_grid_image;
use crate::snapshot::ClientWorldView;
use crate::terrain::TerrainElevation;
use crate::tile_cache::{
    CachedTile, TileEntityQuery, TileSyncFrame, despawn_cached_tile, sync_tile_entities,
    upload_built_tile,
};
use crate::tile_streaming::{TileAsyncQueue, TileStreamPlanner};

/// Client-side tile mesh cache, async streaming queue, LOD hysteresis set, and materials.
#[derive(Resource)]
pub struct GlobeRenderState {
    tiles: HashMap<TileId, CachedTile>,
    previously_split: HashSet<TileId>,
    stream_queue: TileAsyncQueue,
    baked_planet: Option<PlanetSpec>,
    baked_terrain_revision: Option<u64>,
    last_origin: Option<LocalOrigin>,
    applied_debug_overlay: bool,
    frame_counter: u64,
    terrain_material: Handle<StandardMaterial>,
    water_material: Handle<StandardMaterial>,
    face_materials: [Handle<StandardMaterial>; 6],
    /// Current LOD configuration used by the client renderer.
    pub lod_config: LodConfig,
    /// Number of desired leaf tiles selected on screen this frame.
    pub active_tile_count: usize,
    /// Total number of cached tiles currently resident in GPU memory.
    pub cached_tile_count: usize,
    /// Number of in-flight async background tile build tasks.
    pub pending_task_count: usize,
    /// Minimum LOD among active tiles.
    pub min_lod: u8,
    /// Maximum LOD among active tiles.
    pub max_lod: u8,
    /// When `true`, switches tiles to cube-face grid materials and renders seam gizmos.
    pub debug_overlay: bool,
}

impl GlobeRenderState {
    fn initialize(images: &mut Assets<Image>, materials: &mut Assets<StandardMaterial>) -> Self {
        let terrain_material = materials.add(StandardMaterial {
            base_color: Color::WHITE,
            perceptual_roughness: 0.92,
            reflectance: 0.12,
            ..default()
        });
        let water_material = materials.add(StandardMaterial {
            base_color: Color::WHITE,
            perceptual_roughness: 0.16,
            reflectance: 0.65,
            metallic: 0.02,
            ..default()
        });

        let grid_texture = images.add(build_tile_grid_image());
        let face_colors = [
            Color::srgb(0.28, 0.48, 0.62), // +X
            Color::srgb(0.32, 0.56, 0.50), // -X
            Color::srgb(0.58, 0.62, 0.68), // +Y (polar)
            Color::srgb(0.54, 0.58, 0.64), // -Y (polar)
            Color::srgb(0.34, 0.52, 0.42), // +Z
            Color::srgb(0.46, 0.48, 0.36), // -Z
        ];
        let face_materials = face_colors.map(|color| {
            materials.add(StandardMaterial {
                base_color: color,
                base_color_texture: Some(grid_texture.clone()),
                perceptual_roughness: 0.88,
                reflectance: 0.15,
                ..default()
            })
        });

        Self {
            tiles: HashMap::new(),
            previously_split: HashSet::new(),
            stream_queue: TileAsyncQueue::default(),
            baked_planet: None,
            baked_terrain_revision: None,
            last_origin: None,
            applied_debug_overlay: false,
            frame_counter: 0,
            terrain_material,
            water_material,
            face_materials,
            lod_config: LodConfig::standard(),
            active_tile_count: 0,
            cached_tile_count: 0,
            pending_task_count: 0,
            min_lod: 0,
            max_lod: 0,
            debug_overlay: false,
        }
    }

    fn terrain_material_for(&self, tile: TileId) -> Handle<StandardMaterial> {
        if self.debug_overlay {
            self.face_materials[tile.face.index()].clone()
        } else {
            self.terrain_material.clone()
        }
    }
}

/// Startup system initializing tile materials and render cache.
pub fn setup_globe_materials(
    mut commands: Commands,
    mut images: ResMut<Assets<Image>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    let state = GlobeRenderState::initialize(&mut images, &mut materials);
    commands.insert_resource(state);
}

/// Synchronizes visible Bevy tile meshes asynchronously with [`select_tiles_with_elevation`]
/// and updates tile entity transforms in `O(1)` when [`OriginState`] rebases.
#[expect(
    clippy::too_many_arguments,
    clippy::needless_pass_by_value,
    reason = "Bevy ECS system parameters require Res<T> by value"
)]
pub fn sync_globe_tiles(
    mut commands: Commands,
    windows: Query<&Window>,
    world_view: Res<ClientWorldView>,
    terrain: Res<TerrainElevation>,
    cam_state: Res<GlobeCameraState>,
    origin_state: Res<OriginState>,
    mut render_state: ResMut<GlobeRenderState>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut tile_query: TileEntityQuery<'_, '_>,
) {
    let (width_px, height_px) = windows.iter().next().map_or((1280.0_f64, 720.0_f64), |w| {
        (
            f64::from(w.width()).max(1.0),
            f64::from(w.height()).max(1.0),
        )
    });
    let Ok(cam_view) = cam_state.to_camera_view_f64(width_px / height_px, height_px) else {
        return;
    };

    render_state.frame_counter = render_state.frame_counter.wrapping_add(1);
    let planet = world_view.snapshot().planet;
    let terrain_rev = terrain.revision();
    if render_state.baked_planet != Some(planet)
        || render_state.baked_terrain_revision != Some(terrain_rev)
    {
        let drained: Vec<CachedTile> = render_state.tiles.drain().map(|(_, v)| v).collect();
        for cached in drained {
            despawn_cached_tile(&mut commands, &mut meshes, cached);
        }
        render_state.previously_split.clear();
        render_state.stream_queue.in_flight.clear();
        render_state.baked_planet = Some(planet);
        render_state.baked_terrain_revision = Some(terrain_rev);
        render_state.last_origin = None;
    }

    let Ok(selection) = select_tiles_with_elevation(
        planet,
        &cam_view,
        render_state.lod_config,
        &mut render_state.previously_split,
        terrain.sampler(),
    ) else {
        return;
    };

    apply_tile_streaming_frame(
        &mut commands,
        &mut meshes,
        &mut render_state,
        &mut tile_query,
        planet,
        &terrain,
        origin_state.origin,
        &selection,
    );
}

#[expect(
    clippy::too_many_arguments,
    reason = "streaming synchronization coordinates ECS commands, mesh assets, tile queries, and terrain handles"
)]
fn apply_tile_streaming_frame(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    render_state: &mut GlobeRenderState,
    tile_query: &mut TileEntityQuery<'_, '_>,
    planet: PlanetSpec,
    terrain: &TerrainElevation,
    origin: LocalOrigin,
    selection: &strata_globe::TileSelection,
) {
    let terrain_rev = terrain.revision();
    let frame = render_state.frame_counter;
    render_state.active_tile_count = selection.tiles.len();
    render_state.min_lod = selection.min_lod_selected;
    render_state.max_lod = selection.max_lod_selected;

    let desired: HashMap<TileId, CoarserEdges> = selection
        .tiles
        .iter()
        .map(|t| (t.id, t.coarser_edges))
        .collect();

    render_state
        .stream_queue
        .prune_stale(&desired, planet, terrain_rev);
    let completed = render_state.stream_queue.poll_completed(&selection.tiles);
    for built in completed {
        if built.revision == terrain_rev {
            let terrain_mat = render_state.terrain_material_for(built.tile);
            let water_mat = render_state.water_material.clone();
            upload_built_tile(
                commands,
                meshes,
                &mut render_state.tiles,
                tile_query,
                origin,
                frame,
                terrain_mat,
                water_mat,
                built,
            );
        }
    }

    let cached_edges: HashMap<TileId, CoarserEdges> = render_state
        .tiles
        .iter()
        .map(|(&id, c)| (id, c.coarser_edges))
        .collect();
    let sampler_arc = terrain.sampler_arc();
    let world_arc = terrain.world_arc();
    render_state.stream_queue.dispatch_needed(
        planet,
        terrain_rev,
        &selection.tiles,
        &cached_edges,
        &sampler_arc,
        world_arc.as_ref(),
    );

    let ready: HashSet<TileId> = render_state.tiles.keys().copied().collect();
    let visible = TileStreamPlanner::compute_visible_tiles(&selection.tiles, &ready);
    for (&id, cached) in &mut render_state.tiles {
        if visible.contains(&id) || desired.contains_key(&id) {
            cached.last_used_frame = frame;
        }
    }

    let origin_changed = render_state.last_origin != Some(origin);
    let overlay_changed = render_state.applied_debug_overlay != render_state.debug_overlay;
    sync_tile_entities(
        commands,
        &render_state.tiles,
        tile_query,
        TileSyncFrame {
            origin,
            origin_changed,
            overlay_changed,
        },
        &visible,
        |tile_id| render_state.terrain_material_for(tile_id),
    );
    render_state.last_origin = Some(origin);
    render_state.applied_debug_overlay = render_state.debug_overlay;

    let last_used: HashMap<TileId, u64> = render_state
        .tiles
        .iter()
        .map(|(&id, c)| (id, c.last_used_frame))
        .collect();
    let evictions = render_state
        .stream_queue
        .planner
        .plan_evictions(&last_used, &visible);
    for evict_id in evictions {
        if let Some(removed) = render_state.tiles.remove(&evict_id) {
            despawn_cached_tile(commands, meshes, removed);
        }
    }

    render_state.cached_tile_count = render_state.tiles.len();
    render_state.pending_task_count = render_state.stream_queue.in_flight.len();
}
