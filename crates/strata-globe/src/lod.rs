use std::collections::HashSet;

use crate::coords::EcefMeters;
use crate::culling::{
    TileBounds, is_culled, min_sample_distance, tile_geometric_error_meters,
    validate_selection_inputs,
};
use crate::elevation::{ElevationSampler, FlatElevation};
use crate::error::GlobeError;
use crate::planet::PlanetSpec;
use crate::tile::{TileEdge, TileId};

/// Screen-space error LOD selection and culling configuration.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LodConfig {
    /// Pixel error threshold above which a leaf tile splits into 4 children.
    pub split_threshold_px: f64,
    /// Pixel error threshold below which a previously split tile merges back.
    /// Must be `<= split_threshold_px` to provide hysteresis against flickering.
    pub merge_threshold_px: f64,
    /// Minimum quadtree LOD always maintained over the visible globe.
    pub min_lod: u8,
    /// Terrain detail roughness factor relative to grid cell size.
    pub roughness_ratio: f64,
    /// Whether tiles completely beyond the geometric planet horizon are culled.
    pub enable_horizon_culling: bool,
    /// Whether tiles completely outside the camera frustum are culled.
    pub enable_frustum_culling: bool,
}

impl LodConfig {
    /// Default balanced LOD configuration (`split = 3.5 px`, `merge = 1.75 px`).
    #[must_use]
    pub const fn standard() -> Self {
        Self {
            split_threshold_px: 3.5,
            merge_threshold_px: 1.75,
            min_lod: 2,
            roughness_ratio: 0.15,
            enable_horizon_culling: true,
            enable_frustum_culling: true,
        }
    }
}

impl Default for LodConfig {
    fn default() -> Self {
        Self::standard()
    }
}

/// Camera pose and projection parameters in authoritative `f64` ECEF meters.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CameraViewF64 {
    /// Camera eye position in ECEF meters.
    pub position: EcefMeters,
    /// Unit forward view direction.
    pub forward: EcefMeters,
    /// Unit camera right vector.
    pub right: EcefMeters,
    /// Unit camera up vector.
    pub up: EcefMeters,
    /// Vertical field of view in radians.
    pub fov_y_rad: f64,
    /// Viewport width / height aspect ratio.
    pub aspect: f64,
    /// Viewport height in pixels.
    pub viewport_height_px: f64,
}

impl CameraViewF64 {
    /// Constructs a validated [`CameraViewF64`] from eye, target, and up hint.
    ///
    /// # Errors
    ///
    /// Returns [`GlobeError`] if vectors are non-finite/degenerate or projection
    /// parameters are non-positive.
    pub fn look_at(
        position: EcefMeters,
        target: EcefMeters,
        up_hint: EcefMeters,
        fov_y_rad: f64,
        aspect: f64,
        viewport_height_px: f64,
    ) -> Result<Self, GlobeError> {
        if !position.is_finite() || !target.is_finite() || !up_hint.is_finite() {
            return Err(GlobeError::NonFiniteCoordinate);
        }
        if !fov_y_rad.is_finite()
            || fov_y_rad <= 0.0
            || fov_y_rad >= std::f64::consts::PI
            || !aspect.is_finite()
            || aspect <= 0.0
            || !viewport_height_px.is_finite()
            || viewport_height_px <= 0.0
        {
            return Err(GlobeError::InvalidCameraProjection);
        }
        let forward = (target - position).normalized()?;
        let right = forward.cross(up_hint).normalized()?;
        let up = right.cross(forward).normalized()?;
        Ok(Self {
            position,
            forward,
            right,
            up,
            fov_y_rad,
            aspect,
            viewport_height_px,
        })
    }
}

/// Compact 4-bit indicator of which edges of a tile border a coarser (`LOD - 1`) neighbor.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct CoarserEdges {
    mask: u8,
}

impl CoarserEdges {
    /// All four edges share the same or finer LOD.
    pub const NONE: Self = Self { mask: 0 };

    /// Returns a copy of [`CoarserEdges`] with `edge` set or cleared.
    #[must_use]
    pub const fn with_edge(self, edge: TileEdge, coarser: bool) -> Self {
        let bit = match edge {
            TileEdge::Bottom => 1,
            TileEdge::Right => 2,
            TileEdge::Top => 4,
            TileEdge::Left => 8,
        };
        let mask = if coarser {
            self.mask | bit
        } else {
            self.mask & !bit
        };
        Self { mask }
    }

    /// `v == 0` borders a coarser neighbor.
    #[must_use]
    pub const fn bottom(self) -> bool {
        (self.mask & 1) != 0
    }

    /// `u == 1` borders a coarser neighbor.
    #[must_use]
    pub const fn right(self) -> bool {
        (self.mask & 2) != 0
    }

    /// `v == 1` borders a coarser neighbor.
    #[must_use]
    pub const fn top(self) -> bool {
        (self.mask & 4) != 0
    }

    /// `u == 0` borders a coarser neighbor.
    #[must_use]
    pub const fn left(self) -> bool {
        (self.mask & 8) != 0
    }

    /// Compact 4-bit mask (`bit 0 = bottom`, `1 = right`, `2 = top`, `3 = left`).
    #[must_use]
    pub const fn as_mask(self) -> u8 {
        self.mask
    }

    /// Reconstructs [`CoarserEdges`] from a 4-bit mask.
    #[must_use]
    pub const fn from_mask(mask: u8) -> Self {
        Self { mask: mask & 0x0F }
    }
}

/// A visible leaf tile chosen by LOD selection along with edge-stitching metadata.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SelectedTile {
    /// Quadtree address of the tile.
    pub id: TileId,
    /// Edges bordering a coarser `LOD - 1` neighbor.
    pub coarser_edges: CoarserEdges,
    /// Closest sample distance from the camera to this tile in meters.
    pub distance_meters: f64,
    /// Screen-space geometric error in pixels.
    pub sse_px: f64,
}

/// Complete output of a globe LOD selection pass.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct TileSelection {
    /// Selected leaf tiles sorted deterministically by [`TileId`].
    pub tiles: Vec<SelectedTile>,
    /// Minimum LOD among selected leaves.
    pub min_lod_selected: u8,
    /// Maximum LOD among selected leaves.
    pub max_lod_selected: u8,
}

/// Selects visible globe tiles using screen-space error with split/merge hysteresis
/// and enforces 2:1 quadtree neighbor balance across both intra-face and cube-face seams.
pub fn select_tiles<S: std::hash::BuildHasher + Default>(
    planet: PlanetSpec,
    camera: &CameraViewF64,
    config: LodConfig,
    previously_split: &mut HashSet<TileId, S>,
) -> TileSelection {
    select_tiles_with_elevation(planet, camera, config, previously_split, &FlatElevation)
        .unwrap_or_default()
}

/// Selects visible globe tiles against an [`ElevationSampler`], accounting for
/// displaced relief in tile bounds, horizon culling, sample distances, and SSE.
///
/// # Errors
///
/// Returns [`GlobeError`] if camera projection/vectors, LOD config, or sampler
/// global elevation range contain non-finite or invalid parameters.
pub fn select_tiles_with_elevation<S: std::hash::BuildHasher + Default>(
    planet: PlanetSpec,
    camera: &CameraViewF64,
    config: LodConfig,
    previously_split: &mut HashSet<TileId, S>,
    sampler: &dyn ElevationSampler,
) -> Result<TileSelection, GlobeError> {
    validate_selection_inputs(camera, config, sampler)?;
    let global_range = sampler.global_range();
    let effective_min_lod = config.min_lod.min(planet.max_lod());

    let mut leaves = HashSet::new();
    let mut next_split = HashSet::default();
    let mut stack = TileId::roots().to_vec();
    let sse_scale = camera.viewport_height_px / (2.0 * (camera.fov_y_rad * 0.5).tan());

    while let Some(tile) = stack.pop() {
        let tile_range = sampler.tile_range(planet, tile);
        let bounds = TileBounds::compute(planet, tile, tile_range);
        if is_culled(planet, camera, config, &bounds, global_range) {
            continue;
        }

        let min_dist = min_sample_distance(planet, camera.position, tile, sampler);
        let geom_err = tile_geometric_error_meters(planet, tile, config, tile_range);
        let sse_px = (geom_err / min_dist.max(0.25)) * sse_scale;

        let should_split = if tile.lod >= planet.max_lod() {
            false
        } else if tile.lod < effective_min_lod {
            true
        } else if previously_split.contains(&tile) {
            sse_px >= config.merge_threshold_px
        } else {
            sse_px > config.split_threshold_px
        };

        if should_split && let Some(children) = tile.children() {
            next_split.insert(tile);
            stack.extend(children);
        } else {
            leaves.insert(tile);
        }
    }

    balance_quadtree(&mut leaves, planet.max_lod());
    *previously_split = next_split;

    let mut tiles: Vec<SelectedTile> = leaves
        .iter()
        .copied()
        .map(|id| {
            let coarser_edges = compute_coarser_edges(id, &leaves);
            let tile_range = sampler.tile_range(planet, id);
            let distance_meters = min_sample_distance(planet, camera.position, id, sampler);
            let geom_err = tile_geometric_error_meters(planet, id, config, tile_range);
            let sse_px = (geom_err / distance_meters.max(0.25)) * sse_scale;
            SelectedTile {
                id,
                coarser_edges,
                distance_meters,
                sse_px,
            }
        })
        .collect();
    tiles.sort_by_key(|t| t.id);

    let min_lod_selected = tiles.iter().map(|t| t.id.lod).min().unwrap_or(0);
    let max_lod_selected = tiles.iter().map(|t| t.id.lod).max().unwrap_or(0);

    Ok(TileSelection {
        tiles,
        min_lod_selected,
        max_lod_selected,
    })
}

fn balance_quadtree(leaves: &mut HashSet<TileId>, max_lod: u8) {
    for target_lod in (2..=max_lod).rev() {
        let level_tiles: Vec<TileId> = leaves
            .iter()
            .copied()
            .filter(|t| t.lod == target_lod)
            .collect();
        for tile in level_tiles {
            if !leaves.contains(&tile) {
                continue;
            }
            let Some(parent) = tile.parent() else {
                continue;
            };
            for edge in TileEdge::ALL {
                let neighbor_p = parent.neighbor(edge).tile;
                split_ancestors_to_reach(leaves, neighbor_p);
            }
        }
    }
}

fn split_ancestors_to_reach(leaves: &mut HashSet<TileId>, target: TileId) {
    if leaves.contains(&target) {
        return;
    }
    let mut chain = Vec::new();
    let mut cursor = target;
    while let Some(parent) = cursor.parent() {
        chain.push(parent);
        if leaves.contains(&parent) {
            // Split from the found leaf ancestor down toward `target`.
            while let Some(ancestor) = chain.pop() {
                if leaves.remove(&ancestor)
                    && let Some(children) = ancestor.children()
                {
                    leaves.extend(children);
                }
            }
            return;
        }
        cursor = parent;
    }
}

fn compute_coarser_edges(tile: TileId, leaves: &HashSet<TileId>) -> CoarserEdges {
    if tile.lod == 0 {
        return CoarserEdges::NONE;
    }
    let is_coarser = |edge: TileEdge| -> bool {
        let neighbor = tile.neighbor(edge).tile;
        if leaves.contains(&neighbor) {
            return false;
        }
        neighbor.parent().is_some_and(|p| leaves.contains(&p))
    };
    CoarserEdges::NONE
        .with_edge(TileEdge::Bottom, is_coarser(TileEdge::Bottom))
        .with_edge(TileEdge::Right, is_coarser(TileEdge::Right))
        .with_edge(TileEdge::Top, is_coarser(TileEdge::Top))
        .with_edge(TileEdge::Left, is_coarser(TileEdge::Left))
}
