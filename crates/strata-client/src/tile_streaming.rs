use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use bevy::tasks::futures_lite::future::{block_on, poll_once};
use bevy::tasks::{AsyncComputeTaskPool, Task};
use strata_globe::{CoarserEdges, ElevationSampler, PlanetSpec, SelectedTile, TileId};
use strata_terrain::TerrainWorld;

use crate::tile_mesh_build::{BuiltTileMeshes, build_tile_meshes};

/// Maximum number of background tile build jobs dispatched per frame.
pub const MAX_DISPATCH_PER_FRAME: usize = 12;

/// Maximum number of completed tile meshes uploaded to GPU `Assets<Mesh>` per frame.
pub const MAX_UPLOAD_PER_FRAME: usize = 8;

/// Maximum number of cached tile entries retained in the client LRU cache.
pub const MAX_CACHED_TILES: usize = 512;

/// Per-frame streaming and cache-capacity budgets.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TileStreamBudget {
    /// Maximum new async tile builds started per frame.
    pub dispatch_per_frame: usize,
    /// Maximum completed tiles uploaded to Bevy assets per frame.
    pub upload_per_frame: usize,
    /// Maximum cached tiles retained before LRU eviction of hidden tiles.
    pub cache_capacity: usize,
}

impl Default for TileStreamBudget {
    fn default() -> Self {
        Self {
            dispatch_per_frame: MAX_DISPATCH_PER_FRAME,
            upload_per_frame: MAX_UPLOAD_PER_FRAME,
            cache_capacity: MAX_CACHED_TILES,
        }
    }
}

/// Pure-logic tile streaming planner handling dispatch prioritization,
/// ancestor/descendant visibility fallback, stale task checks, and LRU eviction.
#[derive(Debug, Clone, Copy, Default)]
pub struct TileStreamPlanner {
    /// Active streaming budgets.
    pub budget: TileStreamBudget,
}

impl TileStreamPlanner {
    /// Creates a planner with explicit streaming budgets.
    #[must_use]
    pub const fn new(budget: TileStreamBudget) -> Self {
        Self { budget }
    }

    /// Selects up to `budget.dispatch_per_frame` tiles needing build or edge re-stitching,
    /// prioritized closest-to-camera first.
    #[must_use]
    pub fn plan_dispatches(
        &self,
        desired: &[SelectedTile],
        cached_edges: &HashMap<TileId, CoarserEdges>,
        in_flight_edges: &HashMap<TileId, CoarserEdges>,
    ) -> Vec<SelectedTile> {
        let mut candidates: Vec<SelectedTile> = desired
            .iter()
            .copied()
            .filter(|sel| {
                let needs_build = cached_edges
                    .get(&sel.id)
                    .is_none_or(|&edges| edges != sel.coarser_edges);
                if !needs_build {
                    return false;
                }
                let already_in_flight = in_flight_edges
                    .get(&sel.id)
                    .is_some_and(|&edges| edges == sel.coarser_edges);
                !already_in_flight
            })
            .collect();

        candidates.sort_by(|a, b| {
            a.distance_meters
                .total_cmp(&b.distance_meters)
                .then_with(|| a.id.cmp(&b.id))
        });
        candidates.truncate(self.budget.dispatch_per_frame);
        candidates
    }

    /// Computes the non-overlapping subset of `ready` cached tiles that should be visible
    /// this frame, keeping ready ancestors visible while finer descendants build and
    /// keeping ready descendants visible while a coarser parent builds.
    #[must_use]
    pub fn compute_visible_tiles(
        desired: &[SelectedTile],
        ready: &HashSet<TileId>,
    ) -> HashSet<TileId> {
        if ready.is_empty() {
            return HashSet::new();
        }
        let mut visible = HashSet::with_capacity(desired.len());

        for sel in desired {
            let tile = sel.id;
            if ready.contains(&tile) {
                visible.insert(tile);
                continue;
            }
            if let Some(anc) = nearest_ready_ancestor(tile, ready) {
                visible.insert(anc);
                continue;
            }
            for &cand in ready {
                if is_descendant_of(cand, tile)
                    && !has_ready_intermediate_ancestor(cand, tile, ready)
                {
                    visible.insert(cand);
                }
            }
        }

        // If an ancestor is kept visible to cover an unready sibling, hide any ready
        // strict descendants beneath it until all desired descendants are ready.
        let candidates: Vec<TileId> = visible.iter().copied().collect();
        for tile in candidates {
            let mut cursor = tile.parent();
            while let Some(anc) = cursor {
                if visible.contains(&anc) {
                    visible.remove(&tile);
                    break;
                }
                cursor = anc.parent();
            }
        }

        visible
    }

    /// Selects least-recently-used hidden cached tiles to evict above `budget.cache_capacity`.
    #[must_use]
    pub fn plan_evictions(
        &self,
        last_used: &HashMap<TileId, u64>,
        visible: &HashSet<TileId>,
    ) -> Vec<TileId> {
        if last_used.len() <= self.budget.cache_capacity {
            return Vec::new();
        }
        let excess = last_used.len() - self.budget.cache_capacity;
        let mut hidden: Vec<(u64, TileId)> = last_used
            .iter()
            .filter(|(id, _)| !visible.contains(id))
            .map(|(&id, &frame)| (frame, id))
            .collect();
        hidden.sort_unstable();
        hidden.into_iter().take(excess).map(|(_, id)| id).collect()
    }

    /// Returns `true` when an in-flight background job no longer matches the desired frame state.
    #[must_use]
    pub fn is_stale_task(
        tile: TileId,
        task_edges: CoarserEdges,
        task_planet: PlanetSpec,
        task_revision: u64,
        desired: &HashMap<TileId, CoarserEdges>,
        planet: PlanetSpec,
        revision: u64,
    ) -> bool {
        task_planet != planet
            || task_revision != revision
            || desired.get(&tile).is_none_or(|&e| e != task_edges)
    }
}

/// Active background mesh-building task for a single [`TileId`].
pub struct InFlightTile {
    pub coarser_edges: CoarserEdges,
    pub planet: PlanetSpec,
    pub revision: u64,
    pub task: Task<BuiltTileMeshes>,
}

/// Manages async background tile builds on Bevy's [`AsyncComputeTaskPool`].
pub struct TileAsyncQueue {
    pub planner: TileStreamPlanner,
    pub in_flight: HashMap<TileId, InFlightTile>,
}

impl Default for TileAsyncQueue {
    fn default() -> Self {
        Self {
            planner: TileStreamPlanner::new(TileStreamBudget::default()),
            in_flight: HashMap::new(),
        }
    }
}

impl TileAsyncQueue {
    /// Drops any in-flight tasks whose tile is no longer desired or whose planet/revision/edges changed.
    pub fn prune_stale(
        &mut self,
        desired: &HashMap<TileId, CoarserEdges>,
        planet: PlanetSpec,
        revision: u64,
    ) {
        self.in_flight.retain(|&tile, entry| {
            !TileStreamPlanner::is_stale_task(
                tile,
                entry.coarser_edges,
                entry.planet,
                entry.revision,
                desired,
                planet,
                revision,
            )
        });
    }

    /// Non-blocking poll of completed tasks, prioritized closest-to-camera first,
    /// capped at `planner.budget.upload_per_frame`.
    pub fn poll_completed(&mut self, desired: &[SelectedTile]) -> Vec<BuiltTileMeshes> {
        let mut ordered: Vec<(f64, TileId)> = desired
            .iter()
            .filter(|sel| self.in_flight.contains_key(&sel.id))
            .map(|sel| (sel.distance_meters, sel.id))
            .collect();
        ordered.sort_by(|a, b| a.0.total_cmp(&b.0).then_with(|| a.1.cmp(&b.1)));

        let max_upload = self.planner.budget.upload_per_frame;
        let mut completed = Vec::with_capacity(max_upload.min(ordered.len()));

        for (_, tile_id) in ordered {
            if completed.len() >= max_upload {
                break;
            }
            let ready_mesh = self
                .in_flight
                .get_mut(&tile_id)
                .and_then(|entry| block_on(poll_once(&mut entry.task)));
            if let Some(built) = ready_mesh {
                self.in_flight.remove(&tile_id);
                completed.push(built);
            }
        }
        completed
    }

    /// Dispatches up to `planner.budget.dispatch_per_frame` new async build tasks.
    pub fn dispatch_needed(
        &mut self,
        planet: PlanetSpec,
        revision: u64,
        desired: &[SelectedTile],
        cached_edges: &HashMap<TileId, CoarserEdges>,
        sampler: &Arc<dyn ElevationSampler>,
        world: Option<&Arc<TerrainWorld>>,
    ) {
        let in_flight_edges: HashMap<TileId, CoarserEdges> = self
            .in_flight
            .iter()
            .map(|(&id, entry)| (id, entry.coarser_edges))
            .collect();
        let to_dispatch = self
            .planner
            .plan_dispatches(desired, cached_edges, &in_flight_edges);

        let pool = AsyncComputeTaskPool::get();
        for sel in to_dispatch {
            let sampler_cloned = Arc::clone(sampler);
            let world_cloned = world.map(Arc::clone);
            let coarser = sel.coarser_edges;
            let tile = sel.id;
            let task = pool.spawn(async move {
                build_tile_meshes(
                    planet,
                    tile,
                    coarser,
                    revision,
                    sampler_cloned.as_ref(),
                    world_cloned.as_deref(),
                )
            });
            self.in_flight.insert(
                tile,
                InFlightTile {
                    coarser_edges: coarser,
                    planet,
                    revision,
                    task,
                },
            );
        }
    }
}

fn nearest_ready_ancestor(tile: TileId, ready: &HashSet<TileId>) -> Option<TileId> {
    let mut cursor = tile.parent();
    while let Some(anc) = cursor {
        if ready.contains(&anc) {
            return Some(anc);
        }
        cursor = anc.parent();
    }
    None
}

fn has_ready_intermediate_ancestor(
    descendant: TileId,
    root: TileId,
    ready: &HashSet<TileId>,
) -> bool {
    let mut cursor = descendant.parent();
    while let Some(anc) = cursor {
        if anc.lod <= root.lod {
            break;
        }
        if ready.contains(&anc) {
            return true;
        }
        cursor = anc.parent();
    }
    false
}

/// Returns `true` when `child` lies strictly inside `ancestor`'s quadtree subtree.
#[must_use]
pub fn is_descendant_of(child: TileId, ancestor: TileId) -> bool {
    if child.face != ancestor.face || child.lod <= ancestor.lod {
        return false;
    }
    let shift = child.lod - ancestor.lod;
    if shift >= 32 {
        return false;
    }
    (child.x >> shift) == ancestor.x && (child.y >> shift) == ancestor.y
}
