use crate::coords::EcefMeters;
use crate::error::GlobeError;
use crate::planet::PlanetSpec;
use crate::tile::TileId;

/// Validated inclusive vertical relief bounds `[min_meters, max_meters]` relative
/// to [`PlanetSpec::radius_meters`].
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct ElevationRange {
    /// Minimum signed elevation in meters (negative = below mean sphere radius).
    pub min_meters: f64,
    /// Maximum signed elevation in meters.
    pub max_meters: f64,
}

impl ElevationRange {
    /// Zero-relief spherical surface range (`0.0..=0.0`).
    pub const FLAT: Self = Self {
        min_meters: 0.0,
        max_meters: 0.0,
    };

    /// Creates a validated [`ElevationRange`] with finite bounds satisfying `min_meters <= max_meters`.
    ///
    /// # Errors
    ///
    /// Returns [`GlobeError::InvalidElevationRange`] if either bound is non-finite
    /// or `min_meters > max_meters`.
    pub fn new(min_meters: f64, max_meters: f64) -> Result<Self, GlobeError> {
        if !min_meters.is_finite() || !max_meters.is_finite() || min_meters > max_meters {
            return Err(GlobeError::InvalidElevationRange {
                min_meters,
                max_meters,
            });
        }
        Ok(Self {
            min_meters,
            max_meters,
        })
    }

    /// Returns `true` when both bounds are identically zero (`0.0..=0.0`).
    #[must_use]
    pub const fn is_flat(self) -> bool {
        self.min_meters == 0.0 && self.max_meters == 0.0
    }

    /// Vertical relief span in meters (`max_meters - min_meters`, clamped `>= 0.0`).
    #[must_use]
    pub fn span_meters(self) -> f64 {
        (self.max_meters - self.min_meters).max(0.0)
    }

    /// Clamps `min_meters` (and `max_meters` if needed) so `planet.radius_meters() + min_meters > 0.0`.
    #[must_use]
    pub fn clamp_planet_radius(self, planet: PlanetSpec) -> Self {
        let min_radius = 1.0_f64.min(planet.radius_meters() * 0.5);
        let min_allowed = min_radius - planet.radius_meters();
        let min_meters = if self.min_meters.is_finite() {
            self.min_meters.max(min_allowed)
        } else {
            0.0
        };
        let max_meters = if self.max_meters.is_finite() {
            self.max_meters.max(min_meters)
        } else {
            min_meters
        };
        Self {
            min_meters,
            max_meters,
        }
    }

    /// Conservative minimum planetary radius in meters after clamping to stay strictly positive.
    #[must_use]
    pub fn min_radius_meters(self, planet: PlanetSpec) -> f64 {
        let clamped = self.clamp_planet_radius(planet);
        planet.radius_meters() + clamped.min_meters
    }

    /// Conservative maximum planetary radius in meters after clamping.
    #[must_use]
    pub fn max_radius_meters(self, planet: PlanetSpec) -> f64 {
        let clamped = self.clamp_planet_radius(planet);
        planet.radius_meters() + clamped.max_meters
    }
}

/// Direction-indexed terrain elevation contract shared between tile meshing,
/// LOD culling, and client camera/raycast systems.
///
/// Sampling takes an outward unit vector [`EcefMeters`] rather than face-local UV
/// so shared vertices on intra-face seams, 2-face cube edges, and 3-face corners
/// produce bit-identical elevations.
pub trait ElevationSampler: Send + Sync {
    /// Returns signed elevation in meters above [`PlanetSpec::radius_meters`] at `unit_direction`.
    fn sample_elevation(&self, unit_direction: EcefMeters) -> f64;

    /// Returns the conservative global elevation bounds across the entire planet.
    fn global_range(&self) -> ElevationRange;

    /// Returns conservative elevation bounds for `tile` on `planet`.
    fn tile_range(&self, _planet: PlanetSpec, _tile: TileId) -> ElevationRange {
        self.global_range()
    }
}

/// Zero-relief [`ElevationSampler`] representing an ideal smooth sphere (`0.0 m`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct FlatElevation;

impl ElevationSampler for FlatElevation {
    fn sample_elevation(&self, _unit_direction: EcefMeters) -> f64 {
        0.0
    }

    fn global_range(&self) -> ElevationRange {
        ElevationRange::FLAT
    }
}
