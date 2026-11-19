use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use bevy::tasks::futures_lite::future::{block_on, poll_once};
use bevy::tasks::{AsyncComputeTaskPool, Task};
use strata_globe::{CoarserEdges, ElevationSampler, PlanetSpec, SelectedTile, TileId};
use strata_terrain::TerrainWorld;

use crate::tile_mesh_build::{BuiltTileMeshes, build_tile_meshes_cancellable};
use crate::tile_streaming::TileStreamPlanner;

/// Active background mesh-building task for a single [`TileId`].
pub struct InFlightTile {
    pub planet: PlanetSpec,
    pub revision: u64,
    pub cancelled: Arc<AtomicBool>,
    pub task: Task<Option<BuiltTileMeshes>>,
}

/// Manages async background tile builds on Bevy's [`AsyncComputeTaskPool`].
#[derive(Default)]
pub struct TileAsyncQueue {
    pub planner: TileStreamPlanner,
    pub in_flight: HashMap<TileId, InFlightTile>,
    in_flight_ids_scratch: HashSet<TileId>,
}

impl TileAsyncQueue {
    /// Signals cancellation on all active background tasks and clears the queue.
    pub fn cancel_all(&mut self) {
        for entry in self.in_flight.values() {
            entry.cancelled.store(true, Ordering::Relaxed);
        }
        self.in_flight.clear();
    }

    /// Drops any in-flight tasks whose tile is no longer desired or whose planet/revision changed,
    /// flagging each dropped task for immediate cancellation.
    pub fn prune_stale(
        &mut self,
        desired: &HashMap<TileId, CoarserEdges>,
        planet: PlanetSpec,
        revision: u64,
    ) {
        self.in_flight.retain(|&tile, entry| {
            let keep = !TileStreamPlanner::is_stale_task(
                tile,
                entry.planet,
                entry.revision,
                desired,
                planet,
                revision,
            );
            if !keep {
                entry.cancelled.store(true, Ordering::Relaxed);
            }
            keep
        });
    }

    /// Non-blocking poll of completed tasks, prioritized closest-to-camera first,
    /// capped at `planner.budget.upload_per_frame`.
    pub fn poll_completed(&mut self, desired: &[SelectedTile]) -> Vec<BuiltTileMeshes> {
        if self.in_flight.is_empty() {
            return Vec::new();
        }
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
            if let Some(maybe_built) = ready_mesh {
                self.in_flight.remove(&tile_id);
                if let Some(built) = maybe_built {
                    completed.push(built);
                }
            }
        }
        completed
    }

    /// Dispatches up to `planner.budget.dispatch_per_frame` new async build tasks
    /// while respecting the global `planner.budget.max_in_flight` cap.
    #[expect(
        clippy::too_many_arguments,
        reason = "async task dispatch binds planet, revision, LOD pin floor, desired/cached sets, and terrain handles"
    )]
    pub fn dispatch_needed(
        &mut self,
        planet: PlanetSpec,
        revision: u64,
        min_pinned_lod: u8,
        desired: &[SelectedTile],
        cached_tiles: &HashSet<TileId>,
        sampler: &Arc<dyn ElevationSampler>,
        world: Option<&Arc<TerrainWorld>>,
    ) {
        if self.in_flight.len() >= self.planner.budget.max_in_flight {
            return;
        }

        self.in_flight_ids_scratch.clear();
        self.in_flight_ids_scratch
            .extend(self.in_flight.keys().copied());
        let to_dispatch = self.planner.plan_dispatches(
            desired,
            cached_tiles,
            &self.in_flight_ids_scratch,
            min_pinned_lod,
        );
        if to_dispatch.is_empty() {
            return;
        }

        let pool = AsyncComputeTaskPool::get();
        for sel in to_dispatch {
            let sampler_cloned = Arc::clone(sampler);
            let world_cloned = world.map(Arc::clone);
            let tile = sel.id;
            let cancelled = Arc::new(AtomicBool::new(false));
            let cancelled_worker = Arc::clone(&cancelled);
            let task = pool.spawn(async move {
                build_tile_meshes_cancellable(
                    planet,
                    tile,
                    revision,
                    sampler_cloned.as_ref(),
                    world_cloned.as_deref(),
                    &cancelled_worker,
                )
            });
            self.in_flight.insert(
                tile,
                InFlightTile {
                    planet,
                    revision,
                    cancelled,
                    task,
                },
            );
        }
    }
}
