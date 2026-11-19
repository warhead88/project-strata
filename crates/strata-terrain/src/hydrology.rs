use strata_globe::EcefMeters;

use crate::error::TerrainError;

/// Reference global sea level in meters relative to [`strata_globe::PlanetSpec::radius_meters`].
pub const SEA_LEVEL_METERS: f64 = 0.0;

/// Marine and terrestrial bathymetric elevation zone.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BathymetryZone {
    /// Deep ocean floor (`< -2000.0 m`).
    Abyssal,
    /// Continental slope (`-2000.0 .. -200.0 m`).
    ContinentalSlope,
    /// Shallow continental shelf (`-200.0 .. -12.0 m`).
    ContinentalShelf,
    /// Near-shore littoral/intertidal zone (`-12.0 .. 0.0 m`).
    Littoral,
    /// Emergent land (`>= 0.0 m`).
    Land,
}

/// Classifies `elevation_m` relative to [`SEA_LEVEL_METERS`] into a [`BathymetryZone`].
#[must_use]
pub fn bathymetry_zone(elevation_m: f64) -> BathymetryZone {
    if elevation_m >= SEA_LEVEL_METERS {
        BathymetryZone::Land
    } else if elevation_m >= -12.0 {
        BathymetryZone::Littoral
    } else if elevation_m >= -200.0 {
        BathymetryZone::ContinentalShelf
    } else if elevation_m >= -2000.0 {
        BathymetryZone::ContinentalSlope
    } else {
        BathymetryZone::Abyssal
    }
}

/// Returns `true` when `elevation_m` lies within deepwater port draft bounds (`-200.0..=-12.0 m`).
#[must_use]
pub fn is_deepwater_port_depth(elevation_m: f64) -> bool {
    (-200.0..=-12.0).contains(&elevation_m)
}

/// Stable identifier for a lake or inland sea.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct LakeId(pub u32);

/// Salinity classification for lakes and inland seas.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Salinity {
    /// Freshwater lake.
    Freshwater,
    /// Saline lake or endorheic inland sea (e.g. Caspian Sea).
    Saline,
}

/// Definition of a lake or inland sea.
#[derive(Debug, Clone, PartialEq)]
pub struct LakeDef {
    /// Unique lake identifier.
    pub id: LakeId,
    /// Human-readable lake name.
    pub name: &'static str,
    /// Unit direction of lake center.
    pub center_unit: EcefMeters,
    /// Angular radius in radians.
    pub radius_rad: f64,
    /// Water surface elevation in meters relative to sea level.
    pub surface_elevation_m: f64,
    /// Maximum basin depth in meters below `surface_elevation_m`.
    pub max_depth_m: f64,
    /// Lake salinity.
    pub salinity: Salinity,
}

/// Stable identifier for a river system.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct RiverId(pub u32);

/// Commercial vessel navigability class of a river.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Navigability {
    /// Shallow or steep unnavigable headwaters/stream.
    Unnavigable,
    /// Shallow-draft barge traffic.
    Barge,
    /// Full inland river vessel traffic.
    RiverVessel,
    /// Deep estuarine channel accessible to ocean-going vessels.
    SeaGoingEstuary,
}

/// Single control waypoint along a river from headwaters to mouth.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RiverWaypoint {
    /// Outward unit direction on the sphere.
    pub unit_dir: EcefMeters,
    /// Water surface elevation in meters (monotonically non-increasing downstream).
    pub surface_elevation_m: f64,
    /// Channel half-width in meters.
    pub half_width_m: f64,
    /// Thalweg channel depth in meters below `surface_elevation_m`.
    pub channel_depth_m: f64,
    /// Incised valley half-width in meters.
    pub valley_half_width_m: f64,
    /// Mean discharge in cubic meters per second.
    pub discharge_m3s: f64,
}

/// Definition of a river polyline from headwaters to mouth.
#[derive(Debug, Clone, PartialEq)]
pub struct RiverDef {
    /// Unique river identifier.
    pub id: RiverId,
    /// Human-readable river name.
    pub name: &'static str,
    /// Navigability classification.
    pub navigability: Navigability,
    /// Ordered waypoints from headwaters (`0`) to mouth (`len - 1`).
    pub waypoints: Vec<RiverWaypoint>,
}

impl RiverDef {
    /// Constructs a validated [`RiverDef`], enforcing `waypoints.len() >= 2` and
    /// monotonically non-increasing `surface_elevation_m` from headwaters to mouth.
    ///
    /// # Errors
    ///
    /// Returns [`TerrainError::InvalidRiverWaypoints`] if the river has fewer than
    /// 2 waypoints or an upstream-to-downstream elevation increase.
    pub fn new(
        id: RiverId,
        name: &'static str,
        navigability: Navigability,
        waypoints: Vec<RiverWaypoint>,
    ) -> Result<Self, TerrainError> {
        if waypoints.len() < 2 {
            return Err(TerrainError::InvalidRiverWaypoints {
                name,
                reason: "river must have at least 2 waypoints",
            });
        }
        for pair in waypoints.windows(2) {
            if pair[1].surface_elevation_m > pair[0].surface_elevation_m {
                return Err(TerrainError::InvalidRiverWaypoints {
                    name,
                    reason: "river waypoint surface_elevation_m must be monotonically non-increasing",
                });
            }
        }
        Ok(Self {
            id,
            name,
            navigability,
            waypoints,
        })
    }
}

/// Classification of the water body at a sampled surface point.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum WaterBodyKind {
    /// Dry emergent land.
    Dry,
    /// Global ocean or connected marginal sea.
    Ocean,
    /// Inland lake or endorheic sea.
    Lake(LakeId),
    /// Flowing river channel.
    River(RiverId),
}

/// Point-sampled hydrological state returned by [`crate::TerrainWorld::sample_water`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct WaterSample {
    /// Type and identity of water body at this location.
    pub kind: WaterBodyKind,
    /// Water surface elevation in meters (or terrain elevation when `Dry`).
    pub surface_elevation_m: f64,
    /// Water column depth in meters (`0.0` when `Dry`).
    pub depth_m: f64,
    /// Volumetric discharge in `m^3/s` (`0.0` outside rivers).
    pub discharge_m3s: f64,
    /// River navigability classification (`Unnavigable` outside rivers).
    pub navigability: Navigability,
    /// `true` for freshwater rivers/lakes (and dry land), `false` for ocean or saline lakes.
    pub is_freshwater: bool,
}
