use strata_globe::EcefMeters;

use crate::error::TerrainError;
use crate::hydrology::{BathymetryZone, LakeDef, RiverDef, bathymetry_zone};
use crate::query::{
    SiteAssessment, TerrainSample, build_strata_column_for_unit, build_terrain_sample_for_unit,
};
use crate::site_assessment::assess_site_impl;
use crate::strata::{StrataColumn, StratumLayer};
use crate::terrain_world::TerrainWorld;

impl TerrainWorld {
    /// Samples the stack-allocated vertical [`StrataColumn`] at `unit_dir` (zero heap allocations).
    #[must_use]
    pub fn sample_strata_column(&self, unit_dir: EcefMeters) -> StrataColumn {
        let unit = unit_dir
            .normalized()
            .unwrap_or_else(|_| EcefMeters::from_raw(0.0, 0.0, 1.0));
        let (elev, water, cell) = self.sample_core_unit(unit);
        build_strata_column_for_unit(self, unit, elev, water, cell, self.seed)
    }

    /// Samples the [`StratumLayer`] at absolute `elevation_m` below `unit_dir` (zero heap allocations).
    #[must_use]
    pub fn sample_stratum_at_elevation(
        &self,
        unit_dir: EcefMeters,
        elevation_m: f64,
    ) -> StratumLayer {
        let column = self.sample_strata_column(unit_dir);
        let surface_elev = strata_globe::ElevationSampler::sample_elevation(self, unit_dir);
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
        let (elev, water, cell) = self.sample_core_unit(unit);
        let has_void = self
            .patches
            .has_any_underground_void(unit, self.config.planet.radius_meters());
        build_terrain_sample_for_unit(self, unit, elev, water, cell, has_void)
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
