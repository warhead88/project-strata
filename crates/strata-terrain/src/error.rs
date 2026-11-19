use std::fmt;

use strata_globe::GlobeError;

/// Errors returned by terrain configuration, DEM validation, and hydrology construction.
#[derive(Debug, Clone, PartialEq)]
pub enum TerrainError {
    /// Macro cache resolution must be a power of two in `16..=512`.
    InvalidMacroResolution {
        /// Rejected resolution value.
        resolution: u32,
    },
    /// DEM raster dimensions or buffer length were invalid.
    InvalidDemDimensions {
        /// Raster width in texels.
        width: usize,
        /// Raster height in texels.
        height: usize,
        /// Actual length of the elevation slice.
        len: usize,
    },
    /// DEM raster contained a non-finite elevation value.
    NonFiniteDemValue {
        /// Index of the non-finite texel.
        index: usize,
    },
    /// A coordinate or parameter contained NaN or infinity.
    NonFiniteCoordinate,
    /// A direction vector had zero length where a unit direction is required.
    ZeroMagnitudeVector,
    /// Planet radius is invalid for terrain generation.
    InvalidPlanetRadius {
        /// Rejected planet radius in meters.
        radius_meters: f64,
    },
    /// A river definition violated invariants (empty waypoints or non-monotonic elevation).
    InvalidRiverWaypoints {
        /// River name.
        name: &'static str,
        /// Validation failure reason.
        reason: &'static str,
    },
    /// Underlying [`GlobeError`] propagated from `strata-globe`.
    Globe(GlobeError),
}

impl fmt::Display for TerrainError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidMacroResolution { resolution } => {
                write!(
                    f,
                    "invalid macro cache resolution {resolution}: must be a power of two in 16..=512"
                )
            }
            Self::InvalidDemDimensions { width, height, len } => {
                write!(
                    f,
                    "invalid DEM dimensions {width}x{height} for buffer length {len} (width and height must be >= 2)"
                )
            }
            Self::NonFiniteDemValue { index } => {
                write!(f, "non-finite DEM elevation value at index {index}")
            }
            Self::NonFiniteCoordinate => {
                write!(f, "non-finite coordinate or terrain parameter")
            }
            Self::ZeroMagnitudeVector => {
                write!(f, "cannot normalize a zero-magnitude direction vector")
            }
            Self::InvalidPlanetRadius { radius_meters } => {
                write!(
                    f,
                    "invalid planet radius {radius_meters} m: must be finite and > 0"
                )
            }
            Self::InvalidRiverWaypoints { name, reason } => {
                write!(f, "invalid river '{name}': {reason}")
            }
            Self::Globe(err) => write!(f, "globe error: {err}"),
        }
    }
}

impl std::error::Error for TerrainError {}

impl From<GlobeError> for TerrainError {
    fn from(value: GlobeError) -> Self {
        Self::Globe(value)
    }
}
