use std::collections::{HashMap, HashSet};

use strata_globe::{CoarserEdges, PlanetSpec, SelectedTile, TileId};

/// Hard cap on concurrent background tile builds to stay strictly on efficiency cores.
pub const MAX_IN_FLIGHT_TASKS: usize = 3;

/// Maximum number of background tile build jobs dispatched per frame.
pub const MAX_DISPATCH_PER_FRAME: usize = 3;

/// Maximum number of completed tile meshes uploaded to GPU `Assets<Mesh>` per frame.
pub const MAX_UPLOAD_PER_FRAME: usize = 4;

/// Maximum number of cached tile entries retained in the client LRU cache.
pub const MAX_CACHED_TILES: usize = 768;

/// Per-frame streaming and cache-capacity budgets.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TileStreamBudget {
    /// Maximum new async tile builds started per frame.
    pub dispatch_per_frame: usize,
    /// Maximum completed tiles uploaded to Bevy assets per frame.
    pub upload_per_frame: usize,
    /// Hard cap on total concurrent in-flight background tasks.
    pub max_in_flight: usize,
    /// Maximum cached tiles retained before LRU eviction of hidden tiles.
    pub cache_capacity: usize,
}

impl Default for TileStreamBudget {
    fn default() -> Self {
        Self {
            dispatch_per_frame: MAX_DISPATCH_PER_FRAME,
            upload_per_frame: MAX_UPLOAD_PER_FRAME,
            max_in_flight: MAX_IN_FLIGHT_TASKS,
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
    #[cfg_attr(
        not(test),
        expect(dead_code, reason = "planner constructor exercised in unit tests")
    )]
    #[must_use]
    pub const fn new(budget: TileStreamBudget) -> Self {
        Self { budget }
    }

    /// Selects missing tiles needing build, capped by `budget.dispatch_per_frame` and
    /// `budget.max_in_flight`, prioritizing base-shell tiles (`lod <= min_pinned_lod`)
    /// first and then closest-to-camera.
    ///
    /// Already-cached tiles whose `coarser_edges` changed are NOT rebuilt here; their
    /// indices are swapped in-place on GPU assets.
    #[must_use]
    pub fn plan_dispatches(
        &self,
        desired: &[SelectedTile],
        cached_tiles: &HashSet<TileId>,
        in_flight_tiles: &HashSet<TileId>,
        min_pinned_lod: u8,
    ) -> Vec<SelectedTile> {
        let available_slots = self
            .budget
            .max_in_flight
            .saturating_sub(in_flight_tiles.len());
        let limit = self.budget.dispatch_per_frame.min(available_slots);
        if limit == 0 {
            return Vec::new();
        }

        let mut candidates: Vec<SelectedTile> = desired
            .iter()
            .copied()
            .filter(|sel| !cached_tiles.contains(&sel.id) && !in_flight_tiles.contains(&sel.id))
            .collect();

        let cmp = |a: &SelectedTile, b: &SelectedTile| {
            let a_base = a.id.lod <= min_pinned_lod;
            let b_base = b.id.lod <= min_pinned_lod;
            b_base
                .cmp(&a_base)
                .then_with(|| a.distance_meters.total_cmp(&b.distance_meters))
                .then_with(|| a.id.cmp(&b.id))
        };
        if candidates.len() > limit {
            candidates.select_nth_unstable_by(limit, cmp);
            candidates.truncate(limit);
        }
        candidates.sort_unstable_by(cmp);
        candidates
    }

    /// Computes the non-overlapping subset of `ready` cached tiles that should be visible
    /// this frame into `visible`, reusing scratch allocations.
    pub fn compute_visible_tiles_into(
        desired: &[SelectedTile],
        ready: &HashSet<TileId>,
        visible: &mut HashSet<TileId>,
        scratch: &mut Vec<TileId>,
    ) {
        visible.clear();
        if ready.is_empty() {
            return;
        }
        visible.reserve(desired.len());

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
        scratch.clear();
        scratch.extend(visible.iter().copied());
        for &tile in scratch.iter() {
            let mut cursor = tile.parent();
            while let Some(anc) = cursor {
                if visible.contains(&anc) {
                    visible.remove(&tile);
                    break;
                }
                cursor = anc.parent();
            }
        }
    }

    /// Computes the non-overlapping subset of `ready` cached tiles that should be visible
    /// this frame, keeping ready ancestors visible while finer descendants build and
    /// keeping ready descendants visible while a coarser parent builds.
    #[cfg_attr(
        not(test),
        expect(
            dead_code,
            reason = "allocating visibility wrapper exercised in unit tests"
        )
    )]
    #[must_use]
    pub fn compute_visible_tiles(
        desired: &[SelectedTile],
        ready: &HashSet<TileId>,
    ) -> HashSet<TileId> {
        let mut visible = HashSet::new();
        let mut scratch = Vec::new();
        Self::compute_visible_tiles_into(desired, ready, &mut visible, &mut scratch);
        visible
    }

    /// Selects least-recently-used hidden cached tiles to evict above `budget.cache_capacity`,
    /// never evicting visible tiles or pinned base-shell tiles (`lod <= min_pinned_lod`).
    #[must_use]
    pub fn plan_evictions(
        &self,
        last_used: &HashMap<TileId, u64>,
        visible: &HashSet<TileId>,
        min_pinned_lod: u8,
    ) -> Vec<TileId> {
        if last_used.len() <= self.budget.cache_capacity {
            return Vec::new();
        }
        let excess = last_used.len() - self.budget.cache_capacity;
        let mut hidden: Vec<(u64, TileId)> = last_used
            .iter()
            .filter(|(id, _)| id.lod > min_pinned_lod && !visible.contains(id))
            .map(|(&id, &frame)| (frame, id))
            .collect();
        if hidden.len() > excess {
            hidden.select_nth_unstable(excess);
            hidden.truncate(excess);
        }
        hidden.sort_unstable();
        hidden.into_iter().map(|(_, id)| id).collect()
    }

    /// Returns `true` when an in-flight background job no longer matches the desired frame state.
    ///
    /// Edge-stitching mask changes do NOT invalidate in-flight tasks because vertex attributes
    /// are independent of `coarser_edges` and indices are attached at upload time.
    #[must_use]
    pub fn is_stale_task(
        tile: TileId,
        task_planet: PlanetSpec,
        task_revision: u64,
        desired: &HashMap<TileId, CoarserEdges>,
        planet: PlanetSpec,
        revision: u64,
    ) -> bool {
        task_planet != planet || task_revision != revision || !desired.contains_key(&tile)
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
