use std::f64::consts::FRAC_PI_2;

use crate::error::GlobeError;

/// Mean radius of Earth in meters.
pub const EARTH_MEAN_RADIUS_METERS: f64 = 6_371_000.0;

/// Default maximum quadtree LOD for globe tiles.
pub const DEFAULT_MAX_LOD: u8 = 18;

/// Hard upper bound on quadtree LOD so `1_u32 << lod` and tile addressing stay exact.
pub const MAX_SUPPORTED_LOD: u8 = 22;

/// Default number of mesh grid cells along each edge of a tile.
pub const DEFAULT_GRID_SEGMENTS: u32 = 16;

/// Validated geometric specification of a spherical planet.
///
/// All projection, tiling, LOD, and tessellation formulas derive their physical
/// scale from [`PlanetSpec`] rather than hardcoding Earth's radius.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PlanetSpec {
    radius_meters: f64,
    max_lod: u8,
    grid_segments: u32,
}

impl PlanetSpec {
    /// Creates a validated [`PlanetSpec`].
    ///
    /// # Errors
    ///
    /// Returns [`GlobeError`] if `radius_meters` is not finite and positive,
    /// `max_lod > MAX_SUPPORTED_LOD`, or `grid_segments < 4` or odd.
    pub fn new(radius_meters: f64, max_lod: u8, grid_segments: u32) -> Result<Self, GlobeError> {
        if !radius_meters.is_finite() || radius_meters <= 0.0 {
            return Err(GlobeError::InvalidRadius { radius_meters });
        }
        if max_lod > MAX_SUPPORTED_LOD {
            return Err(GlobeError::InvalidMaxLod {
                max_lod,
                supported_max: MAX_SUPPORTED_LOD,
            });
        }
        if grid_segments < 4 || !grid_segments.is_multiple_of(2) {
            return Err(GlobeError::InvalidGridResolution {
                segments: grid_segments,
            });
        }
        Ok(Self {
            radius_meters,
            max_lod,
            grid_segments,
        })
    }

    /// Returns the default Earth specification (`R = 6_371_000.0 m`, `max_lod = 18`, `grid_segments = 16`).
    #[must_use]
    pub const fn earth() -> Self {
        Self {
            radius_meters: EARTH_MEAN_RADIUS_METERS,
            max_lod: DEFAULT_MAX_LOD,
            grid_segments: DEFAULT_GRID_SEGMENTS,
        }
    }

    /// Creates a [`PlanetSpec`] with the given radius and default LOD / grid settings.
    ///
    /// # Errors
    ///
    /// Returns [`GlobeError::InvalidRadius`] if `radius_meters` is not finite and positive.
    pub fn with_radius(radius_meters: f64) -> Result<Self, GlobeError> {
        Self::new(radius_meters, DEFAULT_MAX_LOD, DEFAULT_GRID_SEGMENTS)
    }

    /// Mean sphere radius in meters.
    #[must_use]
    pub const fn radius_meters(self) -> f64 {
        self.radius_meters
    }

    /// Maximum tile quadtree depth.
    #[must_use]
    pub const fn max_lod(self) -> u8 {
        self.max_lod
    }

    /// Number of grid segments per tile edge (always even and `>= 4`).
    #[must_use]
    pub const fn grid_segments(self) -> u32 {
        self.grid_segments
    }

    /// Arc length in meters across one cube face along its center meridian/equator.
    #[must_use]
    pub fn face_arc_length_meters(self) -> f64 {
        FRAC_PI_2 * self.radius_meters
    }

    /// Approximate arc width in meters of a tile at `lod` near the cube-face center.
    #[must_use]
    pub fn tile_arc_length_meters(self, lod: u8) -> f64 {
        let clamped_lod = lod.min(MAX_SUPPORTED_LOD);
        let tiles_per_side = f64::from(1_u32 << clamped_lod);
        self.face_arc_length_meters() / tiles_per_side
    }

    /// Approximate arc width in meters of a single mesh quad cell at `lod`.
    #[must_use]
    pub fn cell_arc_length_meters(self, lod: u8) -> f64 {
        self.tile_arc_length_meters(lod) / f64::from(self.grid_segments)
    }

    /// Exact maximum radial sagitta (`surface_sag_m`) in meters between a mesh triangle
    /// at `lod` and the ideal sphere surface.
    ///
    /// Computed via `2 * R * sin^2(theta_diag / 4)` to avoid precision loss at small angles.
    #[must_use]
    pub fn tile_surface_sagitta_meters(self, lod: u8) -> f64 {
        let clamped_lod = lod.min(MAX_SUPPORTED_LOD);
        let tiles_per_side = f64::from(1_u32 << clamped_lod);
        let cell_angle = FRAC_PI_2 / (tiles_per_side * f64::from(self.grid_segments));
        let half_diag_angle = cell_angle * std::f64::consts::SQRT_2 * 0.5;
        let sin_half = (half_diag_angle * 0.5).sin();
        2.0 * self.radius_meters * sin_half * sin_half
    }

    /// Effective world-space geometric error in meters for a tile at `lod`.
    ///
    /// Combines spherical curvature sagitta with a configurable terrain-detail scale
    /// (`roughness_ratio * cell_arc_length_meters`) so LOD refinement continues into
    /// local scale even where sphere curvature alone has dropped below 1 mm.
    #[must_use]
    pub fn lod_geometric_error_meters(self, lod: u8, roughness_ratio: f64) -> f64 {
        let sagitta = self.tile_surface_sagitta_meters(lod);
        let detail = roughness_ratio.max(0.0) * self.cell_arc_length_meters(lod);
        sagitta.max(detail)
    }
}

impl Default for PlanetSpec {
    fn default() -> Self {
        Self::earth()
    }
}
