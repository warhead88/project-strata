use std::fmt;

/// Errors returned by validated globe, coordinate, and snapshot operations.
#[derive(Debug, Clone, PartialEq)]
pub enum GlobeError {
    /// Planet radius must be finite and strictly positive.
    InvalidRadius {
        /// The rejected radius in meters.
        radius_meters: f64,
    },
    /// Maximum LOD exceeds the supported quadtree depth.
    InvalidMaxLod {
        /// Requested maximum LOD.
        max_lod: u8,
        /// Maximum supported LOD.
        supported_max: u8,
    },
    /// Tile grid segment count must be even and at least 4.
    InvalidGridResolution {
        /// Requested segments per tile edge.
        segments: u32,
    },
    /// Tile `(x, y)` coordinates are out of range for `lod`.
    InvalidTileCoord {
        /// Tile LOD level.
        lod: u8,
        /// Column index.
        x: u32,
        /// Row index.
        y: u32,
    },
    /// A coordinate or camera parameter contained NaN or infinity.
    NonFiniteCoordinate,
    /// A direction or position vector had zero length where a unit vector is required.
    ZeroMagnitudeVector,
    /// Camera field-of-view, aspect ratio, or viewport height was non-positive or invalid.
    InvalidCameraProjection,
    /// Snapshot contained a duplicate stable [`crate::PointId`].
    DuplicatePointId {
        /// Duplicate point identifier.
        id: u64,
    },
    /// Elevation range bounds were non-finite or inverted (`min_meters > max_meters`).
    InvalidElevationRange {
        /// Minimum elevation in meters.
        min_meters: f64,
        /// Maximum elevation in meters.
        max_meters: f64,
    },
}

impl fmt::Display for GlobeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidRadius { radius_meters } => {
                write!(
                    f,
                    "invalid planet radius {radius_meters} m: must be finite and > 0"
                )
            }
            Self::InvalidMaxLod {
                max_lod,
                supported_max,
            } => {
                write!(f, "invalid max LOD {max_lod}: must be <= {supported_max}")
            }
            Self::InvalidGridResolution { segments } => {
                write!(
                    f,
                    "invalid tile grid resolution {segments}: must be even and >= 4"
                )
            }
            Self::InvalidTileCoord { lod, x, y } => {
                write!(f, "tile coordinate ({x}, {y}) out of bounds for LOD {lod}")
            }
            Self::NonFiniteCoordinate => {
                write!(f, "non-finite coordinate or parameter")
            }
            Self::ZeroMagnitudeVector => {
                write!(f, "cannot normalize a zero-magnitude vector")
            }
            Self::InvalidCameraProjection => {
                write!(f, "invalid camera projection parameters")
            }
            Self::DuplicatePointId { id } => {
                write!(f, "duplicate authoritative point id {id}")
            }
            Self::InvalidElevationRange {
                min_meters,
                max_meters,
            } => {
                write!(f, "invalid elevation range [{min_meters}, {max_meters}] m")
            }
        }
    }
}

impl std::error::Error for GlobeError {}
