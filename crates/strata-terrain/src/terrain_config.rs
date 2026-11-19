use strata_globe::PlanetSpec;

use crate::dem::DemGrid;
use crate::earth_geography::TerrainSource;
use crate::macro_cache::DEFAULT_MACRO_RESOLUTION;

/// Default seed for [`crate::TerrainWorld::earth_default`].
pub const DEFAULT_EARTH_SEED: u64 = 0x5354_5241_5441_3031;

/// Configuration for constructing a [`crate::TerrainWorld`].
#[derive(Debug, Clone, PartialEq)]
pub struct TerrainConfig {
    pub(crate) source: TerrainSource,
    pub(crate) planet: PlanetSpec,
    pub(crate) macro_resolution: u32,
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
            TerrainSource::Earth { seed } | TerrainSource::Dem { seed, .. } => *seed,
        }
    }
}
