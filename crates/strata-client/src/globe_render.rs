use std::collections::{HashMap, HashSet};

use bevy::prelude::*;
use strata_globe::{
    CoarserEdges, LocalOrigin, LodConfig, PlanetSpec, TileId, TileSelection,
    select_tiles_with_elevation,
};

use crate::camera::{GlobeCameraState, OriginState};
use crate::globe_debug::TerrainDebug;
use crate::globe_materials::{GlobeMaterials, SelectionKey};
use crate::snapshot::ClientWorldView;
use crate::terrain::TerrainElevation;
use crate::tile_async_queue::TileAsyncQueue;
use crate::tile_cache::{
    CachedTile, StitchedIndexTable, TileEntityQuery, TileSyncFrame, despawn_cached_tile,
    restitch_cached_tile, sync_tile_entities, upload_built_tile,
};
use crate::tile_streaming::TileStreamPlanner;

/// Client-side tile mesh cache, async streaming queue, LOD hysteresis set, and materials.
#[derive(Resource)]
pub struct GlobeRenderState {
    tiles: HashMap<TileId, CachedTile>,
    previously_split: HashSet<TileId>,
    stream_queue: TileAsyncQueue,
    index_table: StitchedIndexTable,
    baked_planet: Option<PlanetSpec>,
    baked_terrain_revision: Option<u64>,
    last_origin: Option<LocalOrigin>,
    last_selection_key: Option<SelectionKey>,
    last_selection: TileSelection,
    all_desired_ready: bool,
    desired_scratch: HashMap<TileId, CoarserEdges>,
    ready_scratch: HashSet<TileId>,
    visible_scratch: HashSet<TileId>,
    visible_candidates_scratch: Vec<TileId>,
    last_used_scratch: HashMap<TileId, u64>,
    applied_debug_overlay: bool,
    frame_counter: u64,
    materials: GlobeMaterials,
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
}

impl GlobeRenderState {
    fn initialize(images: &mut Assets<Image>, materials: &mut Assets<StandardMaterial>) -> Self {
        Self {
            tiles: HashMap::new(),
            previously_split: HashSet::new(),
            stream_queue: TileAsyncQueue::default(),
            index_table: StitchedIndexTable::new(PlanetSpec::earth().grid_segments()),
            baked_planet: None,
            baked_terrain_revision: None,
            last_origin: None,
            last_selection_key: None,
            last_selection: TileSelection::default(),
            all_desired_ready: false,
            desired_scratch: HashMap::new(),
            ready_scratch: HashSet::new(),
            visible_scratch: HashSet::new(),
            visible_candidates_scratch: Vec::new(),
            last_used_scratch: HashMap::new(),
            applied_debug_overlay: false,
            frame_counter: 0,
            materials: GlobeMaterials::new(images, materials),
            lod_config: LodConfig::standard(),
            active_tile_count: 0,
            cached_tile_count: 0,
            pending_task_count: 0,
            min_lod: 0,
            max_lod: 0,
        }
    }

    fn terrain_material_for(&self, tile: TileId, debug: bool) -> Handle<StandardMaterial> {
        self.materials.terrain_for(tile, debug)
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
    debug: Res<TerrainDebug>,
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
        render_state.stream_queue.cancel_all();
        if render_state.index_table.grid_segments() != planet.grid_segments() {
            render_state.index_table = StitchedIndexTable::new(planet.grid_segments());
        }
        render_state.baked_planet = Some(planet);
        render_state.baked_terrain_revision = Some(terrain_rev);
        render_state.last_origin = None;
        render_state.last_selection_key = None;
        render_state.all_desired_ready = false;
    }

    let selection_key = SelectionKey::new(&cam_view, planet, terrain_rev, render_state.lod_config);
    let selection_unchanged = render_state.last_selection_key == Some(selection_key);
    let origin_changed = render_state.last_origin != Some(origin_state.origin);
    let overlay_changed = render_state.applied_debug_overlay != debug.0;

    // Zero-work idle fast path: camera, terrain, origin, overlay unchanged and streaming complete.
    if selection_unchanged
        && render_state.stream_queue.in_flight.is_empty()
        && render_state.all_desired_ready
        && !origin_changed
        && !overlay_changed
    {
        return;
    }

    render_state.frame_counter = render_state.frame_counter.wrapping_add(1);

    let selection = if selection_unchanged {
        std::mem::take(&mut render_state.last_selection)
    } else {
        let Ok(sel) = select_tiles_with_elevation(
            planet,
            &cam_view,
            render_state.lod_config,
            &mut render_state.previously_split,
            terrain.sampler(),
        ) else {
            return;
        };
        render_state.last_selection_key = Some(selection_key);
        sel
    };

    apply_tile_streaming_frame(
        &mut commands,
        &mut meshes,
        &mut render_state,
        &mut tile_query,
        planet,
        &terrain,
        origin_state.origin,
        origin_changed,
        debug.0,
        &selection,
    );
    render_state.last_selection = selection;
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
    origin_changed: bool,
    debug: bool,
    selection: &TileSelection,
) {
    let terrain_rev = terrain.revision();
    let frame = render_state.frame_counter;
    let min_pinned_lod = render_state.lod_config.min_lod;
    render_state.active_tile_count = selection.tiles.len();
    render_state.min_lod = selection.min_lod_selected;
    render_state.max_lod = selection.max_lod_selected;

    render_state.desired_scratch.clear();
    render_state
        .desired_scratch
        .extend(selection.tiles.iter().map(|t| (t.id, t.coarser_edges)));

    render_state
        .stream_queue
        .prune_stale(&render_state.desired_scratch, planet, terrain_rev);
    let completed = render_state.stream_queue.poll_completed(&selection.tiles);
    for built in completed {
        if built.revision == terrain_rev {
            let target_edges = render_state
                .desired_scratch
                .get(&built.tile)
                .copied()
                .unwrap_or(CoarserEdges::NONE);
            let indices = render_state.index_table.indices(target_edges);
            let terrain_mat = render_state.terrain_material_for(built.tile, debug);
            let water_mat = render_state.materials.water.clone();
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
                target_edges,
                indices,
            );
        }
    }

    // Zero-rebuild edge stitching: swap indices in-place when only coarser_edges changed.
    for sel in &selection.tiles {
        if let Some(cached) = render_state.tiles.get_mut(&sel.id)
            && cached.coarser_edges != sel.coarser_edges
        {
            let indices = render_state.index_table.indices(sel.coarser_edges);
            restitch_cached_tile(meshes, cached, sel.coarser_edges, indices);
        }
    }

    render_state.ready_scratch.clear();
    render_state
        .ready_scratch
        .extend(render_state.tiles.keys().copied());

    if render_state.stream_queue.in_flight.len()
        < render_state.stream_queue.planner.budget.max_in_flight
    {
        let sampler_arc = terrain.sampler_arc();
        let world_arc = terrain.world_arc();
        render_state.stream_queue.dispatch_needed(
            planet,
            terrain_rev,
            min_pinned_lod,
            &selection.tiles,
            &render_state.ready_scratch,
            &sampler_arc,
            world_arc.as_ref(),
        );
    }

    TileStreamPlanner::compute_visible_tiles_into(
        &selection.tiles,
        &render_state.ready_scratch,
        &mut render_state.visible_scratch,
        &mut render_state.visible_candidates_scratch,
    );

    for (&id, cached) in &mut render_state.tiles {
        if render_state.visible_scratch.contains(&id)
            || render_state.desired_scratch.contains_key(&id)
        {
            cached.last_used_frame = frame;
        }
    }

    sync_tile_entities(
        commands,
        &render_state.tiles,
        tile_query,
        TileSyncFrame {
            origin,
            origin_changed,
            overlay_changed: render_state.applied_debug_overlay != debug,
        },
        &render_state.visible_scratch,
        |tile_id| render_state.terrain_material_for(tile_id, debug),
    );
    render_state.last_origin = Some(origin);
    render_state.applied_debug_overlay = debug;

    evict_excess_tiles(commands, meshes, render_state, min_pinned_lod);
    render_state.all_desired_ready = selection
        .tiles
        .iter()
        .all(|t| render_state.tiles.contains_key(&t.id));
    render_state.cached_tile_count = render_state.tiles.len();
    render_state.pending_task_count = render_state.stream_queue.in_flight.len();
}

fn evict_excess_tiles(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    render_state: &mut GlobeRenderState,
    min_pinned_lod: u8,
) {
    if render_state.tiles.len() <= render_state.stream_queue.planner.budget.cache_capacity {
        return;
    }
    render_state.last_used_scratch.clear();
    render_state.last_used_scratch.extend(
        render_state
            .tiles
            .iter()
            .map(|(&id, c)| (id, c.last_used_frame)),
    );
    let evictions = render_state.stream_queue.planner.plan_evictions(
        &render_state.last_used_scratch,
        &render_state.visible_scratch,
        min_pinned_lod,
    );
    for evict_id in evictions {
        if let Some(removed) = render_state.tiles.remove(&evict_id) {
            despawn_cached_tile(commands, meshes, removed);
        }
    }
}
