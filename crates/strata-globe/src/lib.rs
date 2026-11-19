//! Pure-Rust globe coordinate, cube-sphere projection, LOD selection,
//! watertight tile meshing, and immutable snapshot contract for Project Strata.
//!
//! This crate is strictly `std`-only with zero Bevy, windowing, or GPU dependencies.

pub mod coords;
pub mod cube;
mod culling;
pub mod error;
pub mod lod;
pub mod mesh;
pub mod planet;
pub mod snapshot;
pub mod tile;

pub use coords::{EcefMeters, LocalOrigin, RebasePolicy, RenderVec3, SurfaceCoord};
pub use cube::{CubeFace, FaceBasis, FaceUv, ecef_to_face_uv, face_uv_to_unit};
pub use error::GlobeError;
pub use lod::{CameraViewF64, CoarserEdges, LodConfig, SelectedTile, TileSelection, select_tiles};
pub use mesh::{
    TileMeshF64, TileRenderMesh, build_stitched_indices, build_tile_mesh_f64,
    build_tile_render_mesh,
};
pub use planet::{
    DEFAULT_GRID_SEGMENTS, DEFAULT_MAX_LOD, EARTH_MEAN_RADIUS_METERS, MAX_SUPPORTED_LOD, PlanetSpec,
};
pub use snapshot::{
    AuthoritativePoint, ControlPointKind, GlobeSnapshot, PointId, SnapshotChannel, WorldEpoch,
};
pub use tile::{TileEdge, TileId, TileNeighbor, TileUv, ecef_to_tile_uv, face_uv_to_tile_uv};
