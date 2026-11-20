use std::fmt;

use strata_globe::GlobeError;

use crate::patches::PatchId;

/// Errors returned by terrain configuration, DEM validation, hydrology, patches, and site queries.
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
    /// Embedded or supplied SDEM raster could not be decoded.
    InvalidEarthDem {
        /// Decode failure reason.
        reason: &'static str,
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
    /// A terrain patch parameter failed validation.
    InvalidPatch {
        /// Validation failure reason.
        reason: &'static str,
    },
    /// Requested terrain patch identifier does not exist.
    PatchNotFound {
        /// Missing patch identifier.
        id: PatchId,
    },
    /// Site assessment radius was non-finite or non-positive.
    InvalidSiteRadius {
        /// Rejected assessment radius in meters.
        radius_m: f64,
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
            Self::InvalidEarthDem { reason } => {
                write!(f, "invalid embedded Earth archive: {reason}")
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
            Self::InvalidPatch { reason } => {
                write!(f, "invalid terrain patch: {reason}")
            }
            Self::PatchNotFound { id } => {
                write!(f, "terrain patch {} not found", id.0)
            }
            Self::InvalidSiteRadius { radius_m } => {
                write!(
                    f,
                    "invalid site assessment radius {radius_m} m: must be finite and > 0"
                )
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
