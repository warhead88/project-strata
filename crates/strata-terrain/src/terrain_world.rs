use std::sync::Arc;

use strata_globe::{
    EcefMeters, ElevationRange, ElevationSampler, PlanetSpec, TileId, ecef_to_face_uv_unit,
};

use crate::climate::{Biome, ClimateSample, classify_biome, compute_climate_sample};
use crate::earth_dem::EarthDem;
use crate::earth_geography::{ElevationGridSource, TerrainSource};
use crate::earth_water::EarthWater;
use crate::error::TerrainError;
use crate::hydrology::WaterSample;
use crate::macro_cache::{CubeMacroCache, MacroCell, default_fallback_sample, micro_detail_fast};
use crate::patches::{PatchId, PatchLayer, PatchReceipt, TerrainPatch, TerrainPatchKind};
use crate::query::{RenderVertexSample, TileVertexBase, build_render_vertex_for_sample};
use crate::terrain_config::{DEFAULT_EARTH_SEED, TerrainConfig};
use crate::water_catalog::dem_water;

/// Complete planetary terrain, hydrology, climate, geology, and patch world implementing
/// [`strata_globe::ElevationSampler`] with `O(1)` zero-allocation point queries.
#[derive(Debug, Clone)]
pub struct TerrainWorld {
    pub(crate) config: TerrainConfig,
    pub(crate) seed: u64,
    pub(crate) elevation_source: ElevationGridSource,
    pub(crate) earth_water: Option<Arc<EarthWater>>,
    pub(crate) macro_cache: CubeMacroCache,
    pub(crate) patches: PatchLayer,
}

impl TerrainWorld {
    /// Builds and bakes a [`TerrainWorld`] from `config`.
    ///
    /// # Errors
    ///
    /// Returns [`TerrainError`] if `macro_resolution` is not a power of two in `16..=512`,
    /// the planet radius is invalid, the embedded Earth data fails validation.
    pub fn new(config: TerrainConfig) -> Result<Self, TerrainError> {
        let radius = config.planet.radius_meters();
        if !radius.is_finite() || radius <= 0.0 {
            return Err(TerrainError::InvalidPlanetRadius {
                radius_meters: radius,
            });
        }
        let seed = config.seed();
        let elevation_source = match &config.source {
            TerrainSource::Earth { .. } => ElevationGridSource::Earth(EarthDem::global_embedded()?),
            TerrainSource::Dem { grid, .. } => {
                ElevationGridSource::CustomDem(Arc::new(grid.clone()))
            }
        };
        let earth_water = matches!(config.source, TerrainSource::Earth { .. })
            .then(EarthWater::embedded)
            .transpose()?;
        let macro_cache = CubeMacroCache::bake(
            config.macro_resolution,
            &config.source,
            &elevation_source,
            earth_water.as_deref(),
        )?;
        Ok(Self {
            config,
            seed,
            elevation_source,
            earth_water,
            macro_cache,
            patches: PatchLayer::new(),
        })
    }

    /// Returns the default Earth [`TerrainWorld`].
    ///
    /// # Panics
    ///
    /// Never panics in normal execution because the default resolution, embedded Earth archive is validated.
    #[must_use]
    #[expect(
        clippy::expect_used,
        reason = "default Earth config uses resolution 128 and the validated embedded Earth archive"
    )]
    pub fn earth_default() -> Self {
        Self::new(TerrainConfig::earth(DEFAULT_EARTH_SEED))
            .expect("default Earth terrain configuration is valid")
    }

    /// Target [`PlanetSpec`].
    #[must_use]
    pub const fn planet(&self) -> PlanetSpec {
        self.config.planet
    }

    /// Original [`TerrainConfig`].
    #[must_use]
    pub const fn config(&self) -> &TerrainConfig {
        &self.config
    }

    /// Per-face macro cache resolution `N`.
    #[must_use]
    pub const fn macro_resolution(&self) -> u32 {
        self.macro_cache.resolution()
    }

    /// Monotonic terrain modification revision (starts at `0`, increments on apply/remove).
    #[must_use]
    pub const fn revision(&self) -> u64 {
        self.patches.revision()
    }

    /// All active terrain patches in application order.
    #[must_use]
    pub fn patches(&self) -> &[TerrainPatch] {
        self.patches.patches()
    }

    /// Validates and applies a local terrain modification or subsurface void patch.
    ///
    /// # Errors
    ///
    /// Returns [`TerrainError`] if vectors are non-finite/zero, radii/widths are non-positive,
    /// corridors are degenerate, or void depth bounds are invalid.
    pub fn apply_patch(&mut self, kind: TerrainPatchKind) -> Result<PatchReceipt, TerrainError> {
        let radius_m = self.config.planet.radius_meters();
        let portal_elev = match kind {
            TerrainPatchKind::UndergroundVoid {
                portal_surface_unit: Some(portal),
                ..
            } => portal
                .normalized()
                .map_or(0.0, |u| self.sample_elevation_unit_prevalidated(u)),
            _ => 0.0,
        };
        self.patches.apply_patch(kind, radius_m, |_| portal_elev)
    }

    /// Removes a previously applied patch by [`PatchId`], restoring baseline terrain and updating bounds.
    ///
    /// # Errors
    ///
    /// Returns [`TerrainError::PatchNotFound`] if `id` is not active.
    pub fn remove_patch(&mut self, id: PatchId) -> Result<PatchReceipt, TerrainError> {
        self.patches.remove_patch(id)
    }

    /// Returns the active [`PatchId`] if an underground void encloses `(unit_dir, depth_m)`.
    #[must_use]
    pub fn has_underground_void_at(&self, unit_dir: EcefMeters, depth_m: f64) -> Option<PatchId> {
        let Ok(unit) = unit_dir.normalized() else {
            return None;
        };
        self.patches
            .has_underground_void_at(unit, self.config.planet.radius_meters(), depth_m)
    }

    /// Returns `true` if any active underground void underlies `unit_dir`.
    #[must_use]
    pub fn has_any_underground_void(&self, unit_dir: EcefMeters) -> bool {
        let Ok(unit) = unit_dir.normalized() else {
            return false;
        };
        self.patches
            .has_any_underground_void(unit, self.config.planet.radius_meters())
    }

    /// Fast elevation-only query for callers that already hold a unit direction vector
    /// (for example from [`strata_globe::face_uv_to_unit`]), skipping normalization.
    #[inline]
    #[must_use]
    pub fn sample_elevation_unit_prevalidated(&self, unit: EcefMeters) -> f64 {
        if let Some(water) = &self.earth_water {
            let height =
                self.elevation_source.sample_bilinear(unit) + water.sample(unit).height_offset_m;
            if self.patches.is_empty() {
                return height;
            }
            let (face, uv) = ecef_to_face_uv_unit(unit);
            return self.patches.apply_to_elevation_uv(
                face,
                uv,
                unit,
                self.planet().radius_meters(),
                height,
            );
        }
        let (face, uv) = ecef_to_face_uv_unit(unit);
        let height = self.elevation_source.sample_bilinear(unit);
        let roughness = self.macro_cache.sample_roughness_uv(face, uv);
        let height = height + micro_detail_fast(unit, height, roughness, self.seed);
        self.patches
            .apply_to_elevation_uv(face, uv, unit, self.planet().radius_meters(), height)
    }

    #[inline]
    fn sample_core(&self, unit_dir: EcefMeters) -> (f64, WaterSample, MacroCell) {
        let Ok(unit) = unit_dir.normalized() else {
            return (
                0.0,
                dem_water(0.0),
                MacroCell::from(default_fallback_sample()),
            );
        };
        self.sample_core_unit(unit)
    }

    #[inline]
    pub(crate) fn sample_core_unit(&self, unit: EcefMeters) -> (f64, WaterSample, MacroCell) {
        let (face, uv) = ecef_to_face_uv_unit(unit);
        let cell = self.macro_cache.sample_bilinear_uv(face, uv);
        let base = self.elevation_source.sample_bilinear(unit);
        let (height, water) = if let Some(atlas) = &self.earth_water {
            let point = atlas.sample(unit);
            let height = base + point.height_offset_m;
            (height, atlas.water(point, height))
        } else {
            let height = base + micro_detail_fast(unit, base, f64::from(cell.roughness), self.seed);
            (height, dem_water(height))
        };
        let (height, water) = self.patches.apply_to_sample_uv(
            face,
            uv,
            unit,
            self.planet().radius_meters(),
            height,
            water,
        );
        (height, water, cell)
    }

    /// Samples elevation, hydrology, climate, and biome in a single `O(1)` zero-allocation pass
    /// for single-pass client tile mesh construction.
    #[inline]
    #[must_use]
    pub fn sample_tile_vertex_base(&self, unit_dir: EcefMeters) -> TileVertexBase {
        let (elevation_m, water, cell) = self.sample_core(unit_dir);
        let climate = compute_climate_sample(cell, elevation_m, water);
        let biome = classify_biome(elevation_m, water, climate);
        TileVertexBase {
            elevation_m,
            water,
            climate,
            biome,
        }
    }

    /// Samples the hydrological state at `unit_dir` (`O(1)`, zero heap allocations).
    #[must_use]
    pub fn sample_water(&self, unit_dir: EcefMeters) -> WaterSample {
        self.sample_core(unit_dir).1
    }

    /// Samples climate indicators at `unit_dir` (`O(1)`, zero heap allocations).
    #[must_use]
    pub fn sample_climate(&self, unit_dir: EcefMeters) -> ClimateSample {
        let (elev, water, cell) = self.sample_core(unit_dir);
        compute_climate_sample(cell, elev, water)
    }

    /// Samples the physical-geographic [`Biome`] at `unit_dir` (`O(1)`, zero heap allocations).
    #[must_use]
    pub fn sample_biome(&self, unit_dir: EcefMeters) -> Biome {
        let (elev, water, cell) = self.sample_core(unit_dir);
        let climate = compute_climate_sample(cell, elev, water);
        classify_biome(elev, water, climate)
    }

    /// Samples physical-geographic surface albedo RGB `[0.0, 1.0]^3` at `unit_dir` and `slope_sin`.
    #[must_use]
    pub fn sample_surface_rgb(&self, unit_dir: EcefMeters, slope_sin: f64) -> [f32; 3] {
        self.sample_render_vertex(unit_dir, slope_sin).albedo_rgb
    }

    /// Samples elevation, surface albedo RGB, and hydrology in one `O(1)` zero-allocation pass.
    #[must_use]
    pub fn sample_render_vertex(&self, unit_dir: EcefMeters, slope_sin: f64) -> RenderVertexSample {
        let (elev, water, cell) = self.sample_core(unit_dir);
        build_render_vertex_for_sample(elev, slope_sin, water, cell)
    }
}

impl ElevationSampler for TerrainWorld {
    #[inline]
    fn sample_elevation(&self, unit_direction: EcefMeters) -> f64 {
        let Ok(unit) = unit_direction.normalized() else {
            return self.sample_elevation_unit_prevalidated(EcefMeters::from_raw(0.0, 0.0, 1.0));
        };
        self.sample_elevation_unit_prevalidated(unit)
    }

    fn global_range(&self) -> ElevationRange {
        self.patches
            .expand_global_range(self.macro_cache.global_range())
    }

    fn tile_range(&self, _planet: PlanetSpec, tile: TileId) -> ElevationRange {
        self.patches
            .expand_tile_range(tile, self.macro_cache.tile_range(tile))
    }
}
