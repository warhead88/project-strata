use strata_globe::{EcefMeters, TileId};

use crate::earth_geography::ElevationGridSource;
use crate::macro_cache::DETAIL_OCTAVES;
use crate::{EarthWaterRaster, MacroCell, TerrainPatchKind, TerrainWorld};

/// Elevations in meters with rows ordered north to south and longitude wrapping at 180°.
pub enum SurfaceRaster<'a> {
    /// Pixel-centered embedded Earth heights.
    Earth(&'a [i16]),
    /// Imported grid, sampled from the western edge and both poles.
    Dem(&'a [f32]),
}

/// Borrowed inputs to surface sampling. Consumers may upload these on world changes.
pub struct SurfaceData<'a>(&'a TerrainWorld);

impl TerrainWorld {
    /// Exposes the inputs for rendering the unmodified surface.
    /// Returns `None` when active surface patches need the full CPU sampler.
    #[must_use]
    pub fn surface_data(&self) -> Option<SurfaceData<'_>> {
        self.patches()
            .iter()
            .all(|patch| {
                matches!(
                    patch.kind,
                    TerrainPatchKind::UndergroundVoid {
                        portal_surface_unit: None,
                        ..
                    }
                )
            })
            .then_some(SurfaceData(self))
    }
}

impl<'a> SurfaceData<'a> {
    /// Returns the prepared fine hydrology atlas for Earth.
    #[must_use]
    pub fn earth_water(&self) -> Option<EarthWaterRaster<'a>> {
        self.0.earth_water.as_ref().map(|water| water.raster())
    }

    /// Returns the raster dimensions and height samples.
    #[must_use]
    pub fn raster(&self) -> (usize, usize, SurfaceRaster<'a>) {
        match &self.0.elevation_source {
            ElevationGridSource::Earth(dem) => (
                dem.width(),
                dem.height(),
                SurfaceRaster::Earth(dem.elevations_m()),
            ),
            ElevationGridSource::CustomDem(dem) => (
                dem.width(),
                dem.height(),
                SurfaceRaster::Dem(dem.elevations_m()),
            ),
        }
    }

    /// Returns the Earth shading normal from fixed one-arcminute relief differences.
    /// The result does not depend on the resolution of the rendered mesh.
    #[must_use]
    pub fn earth_shading_normal(&self, unit: EcefMeters, radius_m: f64) -> Option<EcefMeters> {
        match &self.0.elevation_source {
            ElevationGridSource::Earth(dem) => Some(dem.shading_normal(unit, radius_m)),
            ElevationGridSource::CustomDem(_) => None,
        }
    }

    /// Returns the grid resolution and per-face vertex fields (row major, `(N + 1)^2` each).
    #[must_use]
    pub fn macro_grid(&self) -> (u32, &'a [MacroCell]) {
        (self.0.macro_resolution(), &self.0.macro_cache.vertices)
    }

    /// Returns each detail octave's lattice frequency and effective seed.
    /// The signed coordinate range on each axis is `-frequency..=frequency`.
    #[must_use]
    pub fn detail_octaves(&self) -> [(u32, u64); 2] {
        DETAIL_OCTAVES.map(|(frequency, offset)| (frequency, self.0.seed.wrapping_add(offset)))
    }

    /// Returns the source's conservative height bounds at the requested tile (meters).
    #[must_use]
    pub fn height_bounds(&self, tile: TileId) -> [f64; 2] {
        let range = self.0.macro_cache.tile_range(tile);
        [range.min_meters, range.max_meters]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lat_lon_to_unit;

    #[test]
    fn surface_inputs_are_available_again_after_removing_a_geometry_patch()
    -> Result<(), crate::TerrainError> {
        let mut world = TerrainWorld::earth_default();
        let source = world.surface_data();
        assert!(matches!(
            source.map(|data| data.raster()),
            Some((21600, 10800, SurfaceRaster::Earth(_)))
        ));
        let patch = world.apply_patch(TerrainPatchKind::FlattenPad {
            center_unit: lat_lon_to_unit(47.0, 10.0),
            radius_m: 25.0,
            blend_width_m: 10.0,
            target_elevation_m: 1500.0,
        })?;
        assert!(world.surface_data().is_none());
        world.remove_patch(patch.id)?;
        assert!(world.surface_data().is_some());
        Ok(())
    }
}
