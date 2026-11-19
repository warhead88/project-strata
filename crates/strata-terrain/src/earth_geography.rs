use std::sync::Arc;

use strata_globe::EcefMeters;

use crate::dem::DemGrid;
use crate::earth_dem::EarthDem;

/// Source model used to bake [`crate::CubeMacroCache`] and drive world terrain and hydrology.
#[derive(Debug, Clone, PartialEq)]
pub enum TerrainSource {
    /// Real Earth relief backed by the embedded 15-arcmin SRTM15+ DEM plus real hydrology
    /// and seed-deterministic micro-detail on rough terrain.
    Earth {
        /// Deterministic seed for micro-detail noise.
        seed: u64,
    },
    /// Externally supplied equirectangular DEM raster plugged straight into the terrain world.
    Dem {
        /// Validated elevation raster.
        grid: DemGrid,
        /// Deterministic seed for micro-detail noise.
        seed: u64,
    },
}

/// Shared high-resolution elevation grid backing [`crate::TerrainWorld`].
#[derive(Debug, Clone)]
pub(crate) enum ElevationGridSource {
    Earth(Arc<EarthDem>),
    CustomDem(Arc<DemGrid>),
}

impl ElevationGridSource {
    #[inline]
    #[must_use]
    pub(crate) fn sample_bilinear(&self, unit_dir: EcefMeters) -> f64 {
        match self {
            Self::Earth(dem) => dem.sample_bilinear(unit_dir),
            Self::CustomDem(grid) => grid.sample_bilinear(unit_dir),
        }
    }

    #[inline]
    #[must_use]
    pub(crate) fn sample_quad_minmax(&self, unit_dir: EcefMeters) -> (f64, f64) {
        match self {
            Self::Earth(dem) => dem.sample_quad_minmax(unit_dir),
            Self::CustomDem(grid) => grid.sample_quad_minmax(unit_dir),
        }
    }
}

/// Macro-scale terrain and climate sample baked into each macro-cache vertex.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MacroSample {
    /// Macro elevation relative to sea level in meters.
    pub macro_elevation_m: f32,
    /// Dimensionless terrain roughness factor in `[0.0, 1.0]`.
    pub roughness: f32,
    /// Continentality factor in `[0.0, 1.0]` (`0` = open ocean, `1` = deep continental interior).
    pub continentality: f32,
    /// Mean annual surface temperature in degrees Celsius.
    pub mean_temp_c: f32,
    /// Mean annual precipitation in millimeters.
    pub annual_precip_mm: f32,
    /// Macro geology province identifier (`0..=7`).
    pub geology_province: u8,
}

/// Converts geodetic `(lat_deg, lon_deg)` to a unit [`EcefMeters`] direction using
/// `strata-globe` conventions (`+Y` = North, `+Z` = `0 deg E`, `+X` = `+90 deg E`).
#[must_use]
pub fn lat_lon_to_unit(lat_deg: f64, lon_deg: f64) -> EcefMeters {
    let (sin_lat, cos_lat) = lat_deg.to_radians().sin_cos();
    let (sin_lon, cos_lon) = lon_deg.to_radians().sin_cos();
    EcefMeters::from_raw(cos_lat * sin_lon, sin_lat, cos_lat * cos_lon)
}
