use std::collections::HashMap;

use strata_globe::{EcefMeters, ElevationRange, TileId};

use crate::error::TerrainError;
use crate::hydrology::{Navigability, SEA_LEVEL_METERS, WaterBodyKind, WaterSample};
use crate::patches_index::{lod5_ancestor, lod5_tile_for_unit, rebuild_spatial_index};
use crate::patches_spatial::{Lod5Bucket, PATCH_BUCKET_LOD, PatchEnvelope, PreparedPatch};
use crate::patches_validate::validate_and_prepare;

/// Unique identifier for an applied terrain modification or subsurface void.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct PatchId(pub u64);

/// Parametric local terrain modification or subsurface void specification.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum TerrainPatchKind {
    /// Circular flat pad at `target_elevation_m` with a smooth radial blend skirt.
    FlattenPad {
        /// Outward unit direction of pad center.
        center_unit: EcefMeters,
        /// Interior flat radius in meters (`> 0.0`).
        radius_m: f64,
        /// Exterior smooth blend skirt width in meters (`> 0.0`).
        blend_width_m: f64,
        /// Target elevation inside the pad in meters above planet radius.
        target_elevation_m: f64,
    },
    /// Linearly graded corridor between two surface points for roads, railways, or runways.
    CorridorGrade {
        /// Outward unit direction of corridor start.
        start_unit: EcefMeters,
        /// Outward unit direction of corridor end.
        end_unit: EcefMeters,
        /// Interior half-width in meters (`> 0.0`).
        half_width_m: f64,
        /// Lateral smooth blend skirt width in meters (`> 0.0`).
        blend_width_m: f64,
        /// Target elevation at `start_unit` in meters.
        start_elevation_m: f64,
        /// Target elevation at `end_unit` in meters.
        end_elevation_m: f64,
    },
    /// Open-pit surface excavation depressing terrain by `depth_m` at the center.
    ExcavationPit {
        /// Outward unit direction of pit center.
        center_unit: EcefMeters,
        /// Interior floor radius in meters (`> 0.0`).
        radius_m: f64,
        /// Exterior wall blend width in meters (`> 0.0`).
        blend_width_m: f64,
        /// Excavation depth in meters (`> 0.0`).
        depth_m: f64,
    },
    /// Subsurface cavern, mine drift, or underground bunker complex.
    UndergroundVoid {
        /// Outward unit direction of void center.
        center_unit: EcefMeters,
        /// Horizontal void radius in meters (`> 0.0`).
        radius_m: f64,
        /// Roof depth below local surface in meters (`>= 0.0`).
        top_depth_m: f64,
        /// Floor depth below local surface in meters (`> top_depth_m`).
        bottom_depth_m: f64,
        /// Optional surface portal entrance unit direction (grades a `~25 m` portal pad).
        portal_surface_unit: Option<EcefMeters>,
    },
}

/// Registered terrain patch entry stored in [`crate::TerrainWorld`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TerrainPatch {
    /// Assigned patch identifier.
    pub id: PatchId,
    /// Patch geometry and parameters.
    pub kind: TerrainPatchKind,
}

/// Receipt returned when applying or removing a patch, listing affected `LOD 5` tiles
/// for surgical client mesh cache invalidation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PatchReceipt {
    /// Patch identifier that was created or removed.
    pub id: PatchId,
    /// World terrain revision after this mutation.
    pub revision: u64,
    /// Sorted, deduplicated `LOD 5` cube tiles overlapped by this patch.
    pub affected_tiles: Vec<TileId>,
}

/// Spatial `LOD 5` bucketed terrain patch layer with zero-overhead empty fast path.
#[derive(Debug, Clone)]
pub(crate) struct PatchLayer {
    next_id: u64,
    revision: u64,
    patches: Vec<TerrainPatch>,
    prepared: Vec<PreparedPatch>,
    lod5_buckets: HashMap<TileId, Lod5Bucket>,
    coarse_envelopes: HashMap<TileId, PatchEnvelope>,
    global_envelope: PatchEnvelope,
}

impl PatchLayer {
    #[must_use]
    pub(crate) fn new() -> Self {
        Self {
            next_id: 1,
            revision: 0,
            patches: Vec::new(),
            prepared: Vec::new(),
            lod5_buckets: HashMap::new(),
            coarse_envelopes: HashMap::new(),
            global_envelope: PatchEnvelope::IDENTITY,
        }
    }

    #[must_use]
    pub(crate) const fn revision(&self) -> u64 {
        self.revision
    }

    #[must_use]
    pub(crate) fn patches(&self) -> &[TerrainPatch] {
        &self.patches
    }

    pub(crate) fn apply_patch(
        &mut self,
        kind: TerrainPatchKind,
        planet_radius_m: f64,
        portal_elev_sampler: impl FnOnce(EcefMeters) -> f64,
    ) -> Result<PatchReceipt, TerrainError> {
        let id = PatchId(self.next_id);
        let prepared = validate_and_prepare(id, kind, planet_radius_m, portal_elev_sampler)?;
        let affected_tiles = prepared.affected_tiles.clone();

        self.next_id = self.next_id.saturating_add(1);
        self.revision = self.revision.saturating_add(1);
        self.patches.push(TerrainPatch { id, kind });
        self.prepared.push(prepared);
        self.rebuild_index();

        Ok(PatchReceipt {
            id,
            revision: self.revision,
            affected_tiles,
        })
    }

    pub(crate) fn remove_patch(&mut self, id: PatchId) -> Result<PatchReceipt, TerrainError> {
        let Some(pos) = self.patches.iter().position(|p| p.id == id) else {
            return Err(TerrainError::PatchNotFound { id });
        };
        self.patches.remove(pos);
        let removed = self.prepared.remove(pos);
        self.revision = self.revision.saturating_add(1);
        self.rebuild_index();

        Ok(PatchReceipt {
            id,
            revision: self.revision,
            affected_tiles: removed.affected_tiles,
        })
    }

    #[inline]
    #[must_use]
    pub(crate) fn apply_to_sample(
        &self,
        unit: EcefMeters,
        planet_radius_m: f64,
        base_elev_m: f64,
        water: WaterSample,
    ) -> (f64, WaterSample) {
        if self.patches.is_empty() {
            return (base_elev_m, water);
        }
        let Some(tile) = lod5_tile_for_unit(unit) else {
            return (base_elev_m, water);
        };
        let Some(bucket) = self.lod5_buckets.get(&tile) else {
            return (base_elev_m, water);
        };
        let mut elev = base_elev_m;
        for &idx in &bucket.patch_indices {
            elev = self.prepared[idx].apply_elevation(unit, planet_radius_m, elev);
        }
        let updated_water = update_water_after_patch(elev, water);
        (elev, updated_water)
    }

    #[must_use]
    pub(crate) fn has_underground_void_at(
        &self,
        unit: EcefMeters,
        planet_radius_m: f64,
        depth_m: f64,
    ) -> Option<PatchId> {
        if self.patches.is_empty() || !depth_m.is_finite() || depth_m < 0.0 {
            return None;
        }
        let tile = lod5_tile_for_unit(unit)?;
        let bucket = self.lod5_buckets.get(&tile)?;
        for &idx in &bucket.patch_indices {
            let patch = &self.prepared[idx];
            if patch.contains_void_at(unit, planet_radius_m, depth_m) {
                return Some(patch.id);
            }
        }
        None
    }

    #[must_use]
    pub(crate) fn has_any_underground_void(&self, unit: EcefMeters, planet_radius_m: f64) -> bool {
        if self.patches.is_empty() {
            return false;
        }
        let Some(tile) = lod5_tile_for_unit(unit) else {
            return false;
        };
        let Some(bucket) = self.lod5_buckets.get(&tile) else {
            return false;
        };
        bucket
            .patch_indices
            .iter()
            .any(|&idx| self.prepared[idx].contains_any_void(unit, planet_radius_m))
    }

    #[inline]
    #[must_use]
    pub(crate) fn expand_global_range(&self, base: ElevationRange) -> ElevationRange {
        if self.patches.is_empty() {
            base
        } else {
            self.global_envelope.apply_to_range(base)
        }
    }

    #[inline]
    #[must_use]
    pub(crate) fn expand_tile_range(&self, tile: TileId, base: ElevationRange) -> ElevationRange {
        if self.patches.is_empty() {
            return base;
        }
        let envelope = if tile.lod < PATCH_BUCKET_LOD {
            self.coarse_envelopes.get(&tile).copied()
        } else {
            let bucket_tile = if tile.lod == PATCH_BUCKET_LOD {
                tile
            } else {
                lod5_ancestor(tile)
            };
            self.lod5_buckets.get(&bucket_tile).map(|b| b.envelope)
        };
        match envelope {
            Some(env) => env.apply_to_range(base),
            None => base,
        }
    }

    fn rebuild_index(&mut self) {
        if self.prepared.is_empty() {
            self.lod5_buckets.clear();
            self.coarse_envelopes.clear();
            self.global_envelope = PatchEnvelope::IDENTITY;
            return;
        }
        let (buckets, coarse, global) = rebuild_spatial_index(&self.prepared);
        self.lod5_buckets = buckets;
        self.coarse_envelopes = coarse;
        self.global_envelope = global;
    }
}

#[inline]
fn update_water_after_patch(elev_m: f64, water: WaterSample) -> WaterSample {
    let reclaimed = match water.kind {
        WaterBodyKind::Dry => false,
        WaterBodyKind::Ocean => elev_m >= SEA_LEVEL_METERS,
        WaterBodyKind::Lake(_) | WaterBodyKind::River(_) => elev_m >= water.surface_elevation_m,
    };
    if reclaimed {
        WaterSample {
            kind: WaterBodyKind::Dry,
            surface_elevation_m: elev_m,
            depth_m: 0.0,
            discharge_m3s: 0.0,
            navigability: Navigability::Unnavigable,
            is_freshwater: true,
        }
    } else {
        water
    }
}
