use std::collections::HashSet;
use std::sync::{Arc, RwLock};

use crate::coords::{EcefMeters, SurfaceCoord};
use crate::cube::{CubeFace, FaceUv, face_uv_to_unit};
use crate::error::GlobeError;
use crate::planet::PlanetSpec;
use crate::tile::{TileId, TileUv};

/// Stable identifier of an authoritative world point or entity across ticks and LODs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct PointId(pub u64);

/// Epoch counter incremented whenever a world save/reset replaces the timeline.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct WorldEpoch(pub u64);

/// Classification of authoritative reference/world points.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ControlPointKind {
    /// Point strictly inside a tile interior.
    Interior,
    /// Point lying directly on an intra-face tile seam.
    IntraFaceSeam,
    /// Point lying directly on a 2-face cube edge seam.
    CubeEdge,
    /// Point lying directly on a 3-face cube corner.
    CubeCorner,
    /// Geographic pole (`+Y` or `-Y`).
    Pole,
    /// Antipodal point (`-Z`).
    Antipode,
    /// General simulation/world entity point.
    WorldEntity,
}

/// Immutable authoritative point record published by simulation.
#[derive(Debug, Clone, PartialEq)]
pub struct AuthoritativePoint {
    /// Stable world identifier.
    pub id: PointId,
    /// Human-readable label for inspection and UI.
    pub label: String,
    /// Structural classification.
    pub kind: ControlPointKind,
    /// Authoritative `f64` ECEF position in meters.
    pub position: EcefMeters,
    /// Signed elevation in meters above the planet sphere.
    pub elevation_m: f64,
}

/// Immutable published simulation snapshot consumed by rendering without mutation.
#[derive(Debug, Clone, PartialEq)]
pub struct GlobeSnapshot {
    /// World timeline epoch.
    pub epoch: WorldEpoch,
    /// Authoritative simulation tick number.
    pub tick: u64,
    /// Planet geometry and tessellation specification.
    pub planet: PlanetSpec,
    /// Authoritative world points.
    pub points: Vec<AuthoritativePoint>,
}

impl GlobeSnapshot {
    /// Constructs a validated [`GlobeSnapshot`].
    ///
    /// # Errors
    ///
    /// Returns [`GlobeError`] if any coordinate is non-finite or a [`PointId`] repeats.
    pub fn new(
        epoch: WorldEpoch,
        tick: u64,
        planet: PlanetSpec,
        points: Vec<AuthoritativePoint>,
    ) -> Result<Self, GlobeError> {
        let mut seen = HashSet::with_capacity(points.len());
        for pt in &points {
            if !pt.position.is_finite() || !pt.elevation_m.is_finite() {
                return Err(GlobeError::NonFiniteCoordinate);
            }
            if !seen.insert(pt.id) {
                return Err(GlobeError::DuplicatePointId { id: pt.id.0 });
            }
        }
        Ok(Self {
            epoch,
            tick,
            planet,
            points,
        })
    }

    /// Looks up an authoritative point by its stable [`PointId`].
    #[must_use]
    pub fn point(&self, id: PointId) -> Option<&AuthoritativePoint> {
        self.points.iter().find(|p| p.id == id)
    }

    /// Builds the reference Earth snapshot containing control points across
    /// interior, intra-face seam, 2-face cube edge, 3-face cube corner, pole, and antipode.
    #[must_use]
    pub fn reference_earth() -> Self {
        Self::reference_for_planet(PlanetSpec::earth())
    }

    /// Builds a reference snapshot parameterized for any [`PlanetSpec`].
    #[must_use]
    pub fn reference_for_planet(planet: PlanetSpec) -> Self {
        let r = planet.radius_meters();
        let seam_tile = TileId {
            face: CubeFace::PosZ,
            lod: 4,
            x: 8,
            y: 8,
        };
        let seam_ecef = seam_tile.tile_uv_to_ecef(planet, TileUv::clamped(0.0, 0.5), 0.0);
        let interior_ecef = face_uv_to_unit(CubeFace::PosZ, FaceUv::clamped(0.37, 0.61)).scale(r);
        let edge_ecef = face_uv_to_unit(CubeFace::PosX, FaceUv::clamped(0.0, 0.5)).scale(r);
        let corner_ecef = face_uv_to_unit(CubeFace::PosX, FaceUv::clamped(0.0, 1.0)).scale(r);
        let pole_ecef = EcefMeters::from_raw(0.0, r, 0.0);
        let antipode_ecef = EcefMeters::from_raw(0.0, 0.0, -r);

        // Include a subsurface reference point to validate negative elevation support.
        let subsurface_elevation = (-120.0_f64).max(-0.1 * r);
        let subsurface_ecef = SurfaceCoord {
            latitude_rad: 0.25,
            longitude_rad: 0.4,
            elevation_m: subsurface_elevation,
        }
        .to_ecef(planet)
        .unwrap_or(interior_ecef);

        Self {
            epoch: WorldEpoch(1),
            tick: 0,
            planet,
            points: vec![
                AuthoritativePoint {
                    id: PointId(1),
                    label: "Interior (+Z)".to_owned(),
                    kind: ControlPointKind::Interior,
                    position: interior_ecef,
                    elevation_m: 0.0,
                },
                AuthoritativePoint {
                    id: PointId(2),
                    label: "Tile Seam (+Z LOD4)".to_owned(),
                    kind: ControlPointKind::IntraFaceSeam,
                    position: seam_ecef,
                    elevation_m: 0.0,
                },
                AuthoritativePoint {
                    id: PointId(3),
                    label: "Cube Edge (+X/+Z)".to_owned(),
                    kind: ControlPointKind::CubeEdge,
                    position: edge_ecef,
                    elevation_m: 0.0,
                },
                AuthoritativePoint {
                    id: PointId(4),
                    label: "Cube Corner (+X/+Y/+Z)".to_owned(),
                    kind: ControlPointKind::CubeCorner,
                    position: corner_ecef,
                    elevation_m: 0.0,
                },
                AuthoritativePoint {
                    id: PointId(5),
                    label: "North Pole (+Y)".to_owned(),
                    kind: ControlPointKind::Pole,
                    position: pole_ecef,
                    elevation_m: 0.0,
                },
                AuthoritativePoint {
                    id: PointId(6),
                    label: "Antipode (-Z)".to_owned(),
                    kind: ControlPointKind::Antipode,
                    position: antipode_ecef,
                    elevation_m: 0.0,
                },
                AuthoritativePoint {
                    id: PointId(7),
                    label: "Subsurface Vault (-120 m)".to_owned(),
                    kind: ControlPointKind::WorldEntity,
                    position: subsurface_ecef,
                    elevation_m: subsurface_elevation,
                },
            ],
        }
    }
}

/// Lock-guarded immutable snapshot publication channel separating simulation writes
/// from client reads.
///
/// Readers acquire an [`Arc<GlobeSnapshot>`] that remains completely immutable even
/// after simulation publishes newer ticks.
#[derive(Debug)]
pub struct SnapshotChannel {
    latest: RwLock<Arc<GlobeSnapshot>>,
}

impl SnapshotChannel {
    /// Creates a new channel initialized with `initial` snapshot.
    #[must_use]
    pub fn new(initial: GlobeSnapshot) -> Self {
        Self {
            latest: RwLock::new(Arc::new(initial)),
        }
    }

    /// Publishes a complete new snapshot atomically, replacing the active reference.
    pub fn publish(&self, snapshot: GlobeSnapshot) {
        if let Ok(mut guard) = self.latest.write() {
            *guard = Arc::new(snapshot);
        }
    }

    /// Acquires a read-only handle to the latest published snapshot.
    #[must_use]
    pub fn latest(&self) -> Arc<GlobeSnapshot> {
        match self.latest.read() {
            Ok(guard) => Arc::clone(&guard),
            Err(poisoned) => Arc::clone(&poisoned.into_inner()),
        }
    }
}
