use crate::error::GlobeError;
use crate::planet::PlanetSpec;

/// Authoritative 3D Cartesian coordinates in meters relative to planet center (`f64`).
///
/// Uses a right-handed Y-up globe frame:
/// - `+Y`: North Pole (`lat = +90 deg`)
/// - `+Z`: Prime Meridian at Equator (`lat = 0 deg, lon = 0 deg`)
/// - `+X`: `lat = 0 deg, lon = +90 deg E`
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct EcefMeters {
    /// X coordinate in meters.
    pub x: f64,
    /// Y coordinate in meters.
    pub y: f64,
    /// Z coordinate in meters.
    pub z: f64,
}

impl EcefMeters {
    /// Origin at planet center `(0, 0, 0)`.
    pub const ZERO: Self = Self {
        x: 0.0,
        y: 0.0,
        z: 0.0,
    };

    /// Constructs a validated finite [`EcefMeters`] vector.
    ///
    /// # Errors
    ///
    /// Returns [`GlobeError::NonFiniteCoordinate`] if any component is NaN or infinite.
    pub fn new(x: f64, y: f64, z: f64) -> Result<Self, GlobeError> {
        let point = Self { x, y, z };
        if !point.is_finite() {
            return Err(GlobeError::NonFiniteCoordinate);
        }
        Ok(point)
    }

    /// Constructs an [`EcefMeters`] without validation (for internal exact-math paths).
    #[must_use]
    pub const fn from_raw(x: f64, y: f64, z: f64) -> Self {
        Self { x, y, z }
    }

    /// Returns `[x, y, z]`.
    #[must_use]
    pub const fn to_array(self) -> [f64; 3] {
        [self.x, self.y, self.z]
    }

    /// Returns `true` if all components are finite.
    #[must_use]
    pub const fn is_finite(self) -> bool {
        self.x.is_finite() && self.y.is_finite() && self.z.is_finite()
    }

    /// Dot product in `f64`.
    #[must_use]
    pub fn dot(self, other: Self) -> f64 {
        self.x * other.x + self.y * other.y + self.z * other.z
    }

    /// Cross product `self x other` in `f64`.
    #[must_use]
    pub fn cross(self, other: Self) -> Self {
        Self {
            x: self.y * other.z - self.z * other.y,
            y: self.z * other.x - self.x * other.z,
            z: self.x * other.y - self.y * other.x,
        }
    }

    /// Euclidean norm computed via sorted squared terms so axis permutations
    /// yield bit-identical results across cube faces and 3-face corners.
    #[must_use]
    pub fn norm(self) -> f64 {
        sorted_sum3(self.x * self.x, self.y * self.y, self.z * self.z).sqrt()
    }

    /// Euclidean distance in meters between two ECEF points.
    #[must_use]
    pub fn distance_to(self, other: Self) -> f64 {
        (self - other).norm()
    }

    /// Scalar multiplication.
    #[must_use]
    pub fn scale(self, factor: f64) -> Self {
        Self {
            x: self.x * factor,
            y: self.y * factor,
            z: self.z * factor,
        }
    }

    /// Normalizes the vector to unit length using [`Self::norm`].
    ///
    /// # Errors
    ///
    /// Returns [`GlobeError::NonFiniteCoordinate`] if non-finite or
    /// [`GlobeError::ZeroMagnitudeVector`] if magnitude is zero.
    pub fn normalized(self) -> Result<Self, GlobeError> {
        if !self.is_finite() {
            return Err(GlobeError::NonFiniteCoordinate);
        }
        let len = self.norm();
        if len == 0.0 || !len.is_finite() {
            return Err(GlobeError::ZeroMagnitudeVector);
        }
        Ok(Self {
            x: self.x / len,
            y: self.y / len,
            z: self.z / len,
        })
    }

    /// Signed elevation in meters above `planet.radius_meters()` (negative = underground).
    ///
    /// # Errors
    ///
    /// Returns [`GlobeError`] if the vector is non-finite or zero-length.
    pub fn elevation_above(self, planet: PlanetSpec) -> Result<f64, GlobeError> {
        if !self.is_finite() {
            return Err(GlobeError::NonFiniteCoordinate);
        }
        let r = self.norm();
        if r == 0.0 || !r.is_finite() {
            return Err(GlobeError::ZeroMagnitudeVector);
        }
        Ok(r - planet.radius_meters())
    }
}

/// Camera-relative `f32` render-space vector in meters (never authoritative).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RenderVec3 {
    /// X offset from [`LocalOrigin`] in meters (`f32`).
    pub x: f32,
    /// Y offset from [`LocalOrigin`] in meters (`f32`).
    pub y: f32,
    /// Z offset from [`LocalOrigin`] in meters (`f32`).
    pub z: f32,
}

impl RenderVec3 {
    /// Zero render offset.
    pub const ZERO: Self = Self {
        x: 0.0,
        y: 0.0,
        z: 0.0,
    };

    /// Constructs a [`RenderVec3`].
    #[must_use]
    pub const fn new(x: f32, y: f32, z: f32) -> Self {
        Self { x, y, z }
    }

    /// Returns `[x, y, z]`.
    #[must_use]
    pub const fn to_array(self) -> [f32; 3] {
        [self.x, self.y, self.z]
    }
}

/// Geodetic spherical coordinates `(latitude, longitude, elevation)` relative to a [`PlanetSpec`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SurfaceCoord {
    /// Latitude in radians (`[-pi/2, +pi/2]`).
    pub latitude_rad: f64,
    /// Longitude in radians (`[-pi, +pi]`, positive east of prime meridian `+Z`).
    pub longitude_rad: f64,
    /// Elevation in meters above the sphere surface (negative values represent underground depths).
    pub elevation_m: f64,
}

impl SurfaceCoord {
    /// Creates a [`SurfaceCoord`] from degrees and elevation in meters.
    ///
    /// # Errors
    ///
    /// Returns [`GlobeError::NonFiniteCoordinate`] if any input is non-finite.
    pub fn from_degrees(
        latitude_deg: f64,
        longitude_deg: f64,
        elevation_m: f64,
    ) -> Result<Self, GlobeError> {
        if !latitude_deg.is_finite() || !longitude_deg.is_finite() || !elevation_m.is_finite() {
            return Err(GlobeError::NonFiniteCoordinate);
        }
        Ok(Self {
            latitude_rad: latitude_deg.to_radians(),
            longitude_rad: longitude_deg.to_radians(),
            elevation_m,
        })
    }

    /// Converts spherical coordinates to [`EcefMeters`] for the given `planet`.
    ///
    /// # Errors
    ///
    /// Returns [`GlobeError::NonFiniteCoordinate`] if fields are non-finite.
    pub fn to_ecef(self, planet: PlanetSpec) -> Result<EcefMeters, GlobeError> {
        if !self.latitude_rad.is_finite()
            || !self.longitude_rad.is_finite()
            || !self.elevation_m.is_finite()
        {
            return Err(GlobeError::NonFiniteCoordinate);
        }
        let radius = planet.radius_meters() + self.elevation_m;
        if radius <= 0.0 || !radius.is_finite() {
            return Err(GlobeError::InvalidRadius {
                radius_meters: radius,
            });
        }
        let (sin_lat, cos_lat) = self.latitude_rad.sin_cos();
        let (sin_lon, cos_lon) = self.longitude_rad.sin_cos();
        Ok(EcefMeters {
            x: radius * cos_lat * sin_lon,
            y: radius * sin_lat,
            z: radius * cos_lat * cos_lon,
        })
    }

    /// Converts an [`EcefMeters`] position back to spherical coordinates on `planet`.
    ///
    /// # Errors
    ///
    /// Returns [`GlobeError`] if `ecef` is non-finite or zero-length.
    pub fn from_ecef(planet: PlanetSpec, ecef: EcefMeters) -> Result<Self, GlobeError> {
        let r = ecef.norm();
        if !r.is_finite() {
            return Err(GlobeError::NonFiniteCoordinate);
        }
        if r == 0.0 {
            return Err(GlobeError::ZeroMagnitudeVector);
        }
        let unit_y = (ecef.y / r).clamp(-1.0, 1.0);
        let latitude_rad = unit_y.asin();
        let longitude_rad = ecef.x.atan2(ecef.z);
        Ok(Self {
            latitude_rad,
            longitude_rad,
            elevation_m: r - planet.radius_meters(),
        })
    }
}

/// Floating-origin anchor in authoritative `f64` ECEF meters.
///
/// All `f32` GPU/render positions are produced by subtracting [`LocalOrigin::ecef`]
/// in `f64` before narrowing to `f32`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LocalOrigin {
    /// Anchor position in `f64` ECEF meters.
    pub ecef: EcefMeters,
}

impl LocalOrigin {
    /// Creates a new [`LocalOrigin`] at `ecef`.
    ///
    /// # Errors
    ///
    /// Returns [`GlobeError::NonFiniteCoordinate`] if `ecef` is non-finite.
    pub fn new(ecef: EcefMeters) -> Result<Self, GlobeError> {
        if !ecef.is_finite() {
            return Err(GlobeError::NonFiniteCoordinate);
        }
        Ok(Self { ecef })
    }

    /// Converts an authoritative [`EcefMeters`] point into a camera-relative [`RenderVec3`].
    #[must_use]
    #[expect(
        clippy::cast_possible_truncation,
        reason = "intentional f64->f32 narrowing after high-precision floating-origin subtraction"
    )]
    pub fn to_render_vec3(self, target: EcefMeters) -> RenderVec3 {
        let delta = target - self.ecef;
        RenderVec3::new(delta.x as f32, delta.y as f32, delta.z as f32)
    }

    /// Reconstructs an approximate `f64` ECEF coordinate from a [`RenderVec3`] offset.
    ///
    /// Primarily used for verifying floating-origin precision budgets.
    #[must_use]
    pub fn reconstruct_ecef_f64(self, rel: RenderVec3) -> EcefMeters {
        EcefMeters {
            x: self.ecef.x + f64::from(rel.x),
            y: self.ecef.y + f64::from(rel.y),
            z: self.ecef.z + f64::from(rel.z),
        }
    }
}

impl std::ops::Add for EcefMeters {
    type Output = Self;

    fn add(self, rhs: Self) -> Self::Output {
        Self {
            x: self.x + rhs.x,
            y: self.y + rhs.y,
            z: self.z + rhs.z,
        }
    }
}

impl std::ops::Sub for EcefMeters {
    type Output = Self;

    fn sub(self, rhs: Self) -> Self::Output {
        Self {
            x: self.x - rhs.x,
            y: self.y - rhs.y,
            z: self.z - rhs.z,
        }
    }
}

/// Hysteresis policy controlling when [`LocalOrigin`] rebases to follow the camera.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RebasePolicy {
    /// Rebase threshold in meters when near the surface (default `1_000.0` m).
    pub local_threshold_meters: f64,
    /// Altitude in meters up to which `local_threshold_meters` is strictly enforced.
    pub local_altitude_ceiling_meters: f64,
    /// Fraction of altitude used as rebase threshold in orbital view.
    pub orbital_altitude_fraction: f64,
}

impl RebasePolicy {
    /// Default rebase policy guaranteeing `<= 1 mm` `f32` error within `10 km` of camera.
    #[must_use]
    pub const fn standard() -> Self {
        Self {
            local_threshold_meters: 1_000.0,
            local_altitude_ceiling_meters: 50_000.0,
            orbital_altitude_fraction: 0.02,
        }
    }

    /// Computes the active rebase threshold in meters for the given camera `altitude_meters`.
    #[must_use]
    pub fn threshold_for_altitude(self, altitude_meters: f64) -> f64 {
        let alt = altitude_meters.max(0.0);
        if alt <= self.local_altitude_ceiling_meters {
            self.local_threshold_meters
        } else {
            (alt * self.orbital_altitude_fraction).max(self.local_threshold_meters)
        }
    }

    /// Updates `origin` to `camera_ecef` if distance exceeds the altitude-appropriate threshold.
    ///
    /// Returns `true` when a rebase occurred.
    pub fn maybe_rebase(
        self,
        origin: &mut LocalOrigin,
        camera_ecef: EcefMeters,
        altitude_meters: f64,
    ) -> bool {
        if !camera_ecef.is_finite() {
            return false;
        }
        let threshold = self.threshold_for_altitude(altitude_meters);
        if origin.ecef.distance_to(camera_ecef) > threshold {
            origin.ecef = camera_ecef;
            true
        } else {
            false
        }
    }
}

impl Default for RebasePolicy {
    fn default() -> Self {
        Self::standard()
    }
}

/// Sums three non-negative numbers in ascending order for deterministic,
/// permutation-invariant floating-point precision.
#[inline]
fn sorted_sum3(a: f64, b: f64, c: f64) -> f64 {
    let mut vals = [a, b, c];
    if vals[0] > vals[1] {
        vals.swap(0, 1);
    }
    if vals[1] > vals[2] {
        vals.swap(1, 2);
    }
    if vals[0] > vals[1] {
        vals.swap(0, 1);
    }
    (vals[0] + vals[1]) + vals[2]
}
