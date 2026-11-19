use strata_globe::{EcefMeters, ElevationRange, ElevationSampler, PlanetSpec, TileId};

use crate::climate::{
    Biome, ClimateSample, classify_biome, compute_climate_sample, surface_albedo_rgb,
};
use crate::error::TerrainError;
use crate::hydrology::{
    BathymetryZone, LakeDef, RiverDef, WaterBodyKind, WaterSample, bathymetry_zone,
};
use crate::hydrology_builder::build_hydrology_index;
use crate::hydrology_index::HydrologyIndex;
use crate::macro_cache::{CubeMacroCache, MacroCell, micro_detail_offset};
use crate::patches::{PatchId, PatchLayer, PatchReceipt, TerrainPatch, TerrainPatchKind};
use crate::query::{RenderVertexSample, SiteAssessment, TerrainSample, compute_slope_and_aspect};
use crate::site_assessment::assess_site_impl;
use crate::strata::{GeologyProvince, StrataColumn, StratumLayer};
use crate::strata_derive::{derive_strata_column, derive_surface_material};
use crate::terrain_config::{DEFAULT_EARTH_SEED, TerrainConfig};

/// Complete planetary terrain, hydrology, climate, geology, and patch world implementing
/// [`strata_globe::ElevationSampler`] with `O(1)` zero-allocation point queries.
#[derive(Debug, Clone)]
pub struct TerrainWorld {
    config: TerrainConfig,
    seed: u64,
    hydrology: HydrologyIndex,
    macro_cache: CubeMacroCache,
    patches: PatchLayer,
}

impl TerrainWorld {
    /// Builds and bakes a [`TerrainWorld`] from `config`.
    ///
    /// # Errors
    ///
    /// Returns [`TerrainError`] if `macro_resolution` is not a power of two in `16..=512`,
    /// the planet radius is invalid, or river definitions violate invariants.
    pub fn new(config: TerrainConfig) -> Result<Self, TerrainError> {
        let radius = config.planet.radius_meters();
        if !radius.is_finite() || radius <= 0.0 {
            return Err(TerrainError::InvalidPlanetRadius {
                radius_meters: radius,
            });
        }
        let seed = config.seed();
        let hydrology = build_hydrology_index(config.planet, &config.source)?;
        let macro_cache =
            CubeMacroCache::bake(config.macro_resolution, &config.source, &hydrology)?;
        Ok(Self {
            config,
            seed,
            hydrology,
            macro_cache,
            patches: PatchLayer::new(),
        })
    }

    /// Returns the default Earth [`TerrainWorld`].
    ///
    /// # Panics
    ///
    /// Never panics in normal execution because the default resolution and static Earth
    /// river definitions are validated.
    #[must_use]
    #[expect(
        clippy::expect_used,
        reason = "default Earth config uses resolution 128 and statically validated Earth rivers"
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
                .map_or(0.0, |u| self.sample_elevation(u)),
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

    /// Evaluates elevation, water body, and cached macro cell in a single `O(1)` zero-allocation pass.
    #[inline]
    fn sample_core(&self, unit_dir: EcefMeters) -> (f64, WaterSample, MacroCell) {
        let Ok(unit) = unit_dir.normalized() else {
            let fallback = MacroCell {
                macro_elevation_m: 0.0,
                roughness: 0.0,
                continentality: 0.0,
                mean_temp_c: 15.0,
                annual_precip_mm: 800.0,
                geology_province: 0,
            };
            let (elev, water) = self
                .hydrology
                .carve_and_sample(EcefMeters::from_raw(0.0, 0.0, 1.0), 0.0);
            return (elev, water, fallback);
        };
        let cell = self.macro_cache.sample_bilinear(unit);
        let micro = micro_detail_offset(unit, f64::from(cell.roughness), self.seed);
        let base_elev = f64::from(cell.macro_elevation_m) + micro;
        let (elev, water) = self.hydrology.carve_and_sample(unit, base_elev);
        let (elev, water) =
            self.patches
                .apply_to_sample(unit, self.config.planet.radius_meters(), elev, water);
        (elev, water, cell)
    }

    /// Samples the hydrological state at `unit_dir` (`O(1)`, zero heap allocations).
    #[must_use]
    pub fn sample_water(&self, unit_dir: EcefMeters) -> WaterSample {
        let (_, water, _) = self.sample_core(unit_dir);
        water
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
        let climate = compute_climate_sample(cell, elev, water);
        let biome = classify_biome(elev, water, climate);
        let albedo_rgb = surface_albedo_rgb(elev, slope_sin, water, climate, biome);
        RenderVertexSample {
            elevation_m: elev,
            albedo_rgb,
            water,
        }
    }

    /// Samples the stack-allocated vertical [`StrataColumn`] at `unit_dir` (zero heap allocations).
    #[must_use]
    pub fn sample_strata_column(&self, unit_dir: EcefMeters) -> StrataColumn {
        let unit = unit_dir
            .normalized()
            .unwrap_or_else(|_| EcefMeters::from_raw(0.0, 0.0, 1.0));
        let (elev, water, cell) = self.sample_core(unit);
        let climate = compute_climate_sample(cell, elev, water);
        let biome = classify_biome(elev, water, climate);
        let province = GeologyProvince::from_id(cell.geology_province);
        let (slope_rad, _, _) = compute_slope_and_aspect(self, unit);
        derive_strata_column(
            unit,
            elev,
            slope_rad.sin(),
            water,
            climate,
            biome,
            province,
            self.seed,
        )
    }

    /// Samples the [`StratumLayer`] at absolute `elevation_m` below `unit_dir` (zero heap allocations).
    #[must_use]
    pub fn sample_stratum_at_elevation(
        &self,
        unit_dir: EcefMeters,
        elevation_m: f64,
    ) -> StratumLayer {
        let column = self.sample_strata_column(unit_dir);
        let surface_elev = self.sample_elevation(unit_dir);
        let depth_m = if elevation_m.is_finite() {
            (surface_elev - elevation_m).max(0.0)
        } else {
            0.0
        };
        *column.layer_at_depth(depth_m)
    }

    /// Samples the full physical-geographic [`TerrainSample`] at `unit_dir` (zero heap allocations).
    #[must_use]
    pub fn sample_terrain(&self, unit_dir: EcefMeters) -> TerrainSample {
        let unit = unit_dir
            .normalized()
            .unwrap_or_else(|_| EcefMeters::from_raw(0.0, 0.0, 1.0));
        let (elev, water, cell) = self.sample_core(unit);
        let climate = compute_climate_sample(cell, elev, water);
        let biome = classify_biome(elev, water, climate);
        let province = GeologyProvince::from_id(cell.geology_province);
        let (slope_rad, slope_percent, aspect_rad) = compute_slope_and_aspect(self, unit);
        let surface_material =
            derive_surface_material(elev, slope_rad.sin(), water, climate, biome);
        let permafrost = water.kind == WaterBodyKind::Dry
            && (climate.mean_temp_c < -1.0 || climate.permafrost_prob > 0.15);
        let has_underground_void = self
            .patches
            .has_any_underground_void(unit, self.config.planet.radius_meters());

        TerrainSample {
            elevation_m: elev,
            slope_rad,
            slope_percent,
            aspect_rad,
            roughness: cell.roughness,
            water,
            bathymetry: bathymetry_zone(elev),
            climate,
            biome,
            province,
            surface_material,
            permafrost,
            has_underground_void,
        }
    }

    /// Evaluates aggregate engineering, hydrology, and geographic metrics within `radius_m`
    /// of `center_unit` using 25 deterministic concentric-ring samples.
    ///
    /// # Errors
    ///
    /// Returns [`TerrainError`] if `radius_m` is non-finite/non-positive or `center_unit` is invalid.
    pub fn assess_site(
        &self,
        center_unit: EcefMeters,
        radius_m: f64,
    ) -> Result<SiteAssessment, TerrainError> {
        assess_site_impl(self, center_unit, radius_m)
    }

    /// Classifies `elevation_m` into a [`BathymetryZone`].
    #[must_use]
    pub fn bathymetry_zone(&self, elevation_m: f64) -> BathymetryZone {
        bathymetry_zone(elevation_m)
    }

    /// All river systems registered in this world.
    #[must_use]
    pub fn rivers(&self) -> &[RiverDef] {
        self.hydrology.rivers()
    }

    /// All lakes and inland seas registered in this world.
    #[must_use]
    pub fn lakes(&self) -> &[LakeDef] {
        self.hydrology.lakes()
    }

    /// Returns the macro geology province identifier (`0..=7`) at `unit_dir`.
    #[must_use]
    pub fn geology_province_id(&self, unit_dir: EcefMeters) -> u8 {
        let Ok(unit) = unit_dir.normalized() else {
            return 0;
        };
        self.macro_cache.sample_bilinear(unit).geology_province
    }
}

impl ElevationSampler for TerrainWorld {
    fn sample_elevation(&self, unit_direction: EcefMeters) -> f64 {
        let (elev, _, _) = self.sample_core(unit_direction);
        elev
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
