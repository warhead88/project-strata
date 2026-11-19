use strata_globe::{EcefMeters, ElevationRange, ElevationSampler, PlanetSpec, TileId};

use crate::climate::{
    Biome, ClimateSample, classify_biome, compute_climate_sample, surface_albedo_rgb,
};
use crate::dem::DemGrid;
use crate::earth_geography::TerrainSource;
use crate::error::TerrainError;
use crate::hydrology::{BathymetryZone, LakeDef, RiverDef, WaterSample, bathymetry_zone};
use crate::hydrology_builder::build_hydrology_index;
use crate::hydrology_index::HydrologyIndex;
use crate::macro_cache::{
    CubeMacroCache, DEFAULT_MACRO_RESOLUTION, MacroCell, micro_detail_offset,
};

/// Default seed for [`TerrainWorld::earth_default`].
pub const DEFAULT_EARTH_SEED: u64 = 0x5354_5241_5441_3031;

/// Configuration for constructing a [`TerrainWorld`].
#[derive(Debug, Clone, PartialEq)]
pub struct TerrainConfig {
    source: TerrainSource,
    planet: PlanetSpec,
    macro_resolution: u32,
}

impl TerrainConfig {
    /// Creates an Earth-geography configuration with `seed`, [`PlanetSpec::earth`],
    /// and [`DEFAULT_MACRO_RESOLUTION`].
    #[must_use]
    pub fn earth(seed: u64) -> Self {
        Self {
            source: TerrainSource::Earth { seed },
            planet: PlanetSpec::earth(),
            macro_resolution: DEFAULT_MACRO_RESOLUTION,
        }
    }

    /// Creates a seed-driven procedural planet configuration with [`PlanetSpec::earth`]
    /// and [`DEFAULT_MACRO_RESOLUTION`].
    #[must_use]
    pub fn procedural(seed: u64) -> Self {
        Self {
            source: TerrainSource::Procedural { seed },
            planet: PlanetSpec::earth(),
            macro_resolution: DEFAULT_MACRO_RESOLUTION,
        }
    }

    /// Creates a configuration backed by an imported equirectangular [`DemGrid`].
    #[must_use]
    pub fn from_dem(grid: DemGrid, seed: u64) -> Self {
        Self {
            source: TerrainSource::Dem { grid, seed },
            planet: PlanetSpec::earth(),
            macro_resolution: DEFAULT_MACRO_RESOLUTION,
        }
    }

    /// Sets the target [`PlanetSpec`].
    #[must_use]
    pub const fn with_planet(mut self, planet: PlanetSpec) -> Self {
        self.planet = planet;
        self
    }

    /// Sets the per-face macro cache cell resolution (must be a power of two in `16..=512`).
    #[must_use]
    pub const fn with_macro_resolution(mut self, resolution: u32) -> Self {
        self.macro_resolution = resolution;
        self
    }

    /// Configured terrain source.
    #[must_use]
    pub const fn source(&self) -> &TerrainSource {
        &self.source
    }

    /// Configured [`PlanetSpec`].
    #[must_use]
    pub const fn planet(&self) -> PlanetSpec {
        self.planet
    }

    /// Configured per-face macro cache resolution.
    #[must_use]
    pub const fn macro_resolution(&self) -> u32 {
        self.macro_resolution
    }

    /// Deterministic seed associated with this configuration.
    #[must_use]
    pub const fn seed(&self) -> u64 {
        match &self.source {
            TerrainSource::Earth { seed }
            | TerrainSource::Procedural { seed }
            | TerrainSource::Dem { seed, .. } => *seed,
        }
    }
}

/// Complete planetary terrain, hydrology, climate, and biome world implementing
/// [`strata_globe::ElevationSampler`] with `O(1)` zero-allocation point queries.
#[derive(Debug, Clone)]
pub struct TerrainWorld {
    config: TerrainConfig,
    seed: u64,
    hydrology: HydrologyIndex,
    macro_cache: CubeMacroCache,
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
        let (elev, water, cell) = self.sample_core(unit_dir);
        let climate = compute_climate_sample(cell, elev, water);
        let biome = classify_biome(elev, water, climate);
        surface_albedo_rgb(elev, slope_sin, water, climate, biome)
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
        self.macro_cache.global_range()
    }

    fn tile_range(&self, _planet: PlanetSpec, tile: TileId) -> ElevationRange {
        self.macro_cache.tile_range(tile)
    }
}
