use crate::coords::EcefMeters;
use crate::cube::{CubeFace, FaceUv, ecef_to_face_uv, face_uv_to_unit};
use crate::error::GlobeError;
use crate::planet::{MAX_SUPPORTED_LOD, PlanetSpec};

/// Normalized tile-local coordinates `(u, v)` in `[0.0, 1.0]^2`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TileUv {
    /// Horizontal tile coordinate in `[0.0, 1.0]`.
    pub u: f64,
    /// Vertical tile coordinate in `[0.0, 1.0]`.
    pub v: f64,
}

impl TileUv {
    /// Creates a [`TileUv`] clamped to `[0.0, 1.0]^2`.
    #[must_use]
    pub fn clamped(u: f64, v: f64) -> Self {
        Self {
            u: u.clamp(0.0, 1.0),
            v: v.clamp(0.0, 1.0),
        }
    }
}

/// The four edges of a tile in tile-local UV space.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TileEdge {
    /// `v == 0` edge.
    Bottom,
    /// `u == 1` edge.
    Right,
    /// `v == 1` edge.
    Top,
    /// `u == 0` edge.
    Left,
}

impl TileEdge {
    /// All four edges in counter-clockwise order.
    pub const ALL: [Self; 4] = [Self::Bottom, Self::Right, Self::Top, Self::Left];
}

/// Result of stepping across a [`TileEdge`] to an adjacent same-LOD tile.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TileNeighbor {
    /// Adjacent tile at the same `lod`.
    pub tile: TileId,
    /// Edge on `tile` that touches the source tile.
    pub shared_edge_on_neighbor: TileEdge,
    /// `true` if stepping across this edge crossed a cube-face seam.
    pub crossed_face: bool,
}

/// Unique quadtree tile address on the cube-sphere.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct TileId {
    /// Cube face containing this tile.
    pub face: CubeFace,
    /// Quadtree subdivision level (`0` = whole face).
    pub lod: u8,
    /// Column index in `0..(1 << lod)`.
    pub x: u32,
    /// Row index in `0..(1 << lod)`.
    pub y: u32,
}

impl TileId {
    /// Creates a validated [`TileId`].
    ///
    /// # Errors
    ///
    /// Returns [`GlobeError`] if `lod > MAX_SUPPORTED_LOD` or `x`/`y` are out of bounds.
    pub fn new(face: CubeFace, lod: u8, x: u32, y: u32) -> Result<Self, GlobeError> {
        if lod > MAX_SUPPORTED_LOD {
            return Err(GlobeError::InvalidMaxLod {
                max_lod: lod,
                supported_max: MAX_SUPPORTED_LOD,
            });
        }
        let dim = 1_u32 << lod;
        if x >= dim || y >= dim {
            return Err(GlobeError::InvalidTileCoord { lod, x, y });
        }
        Ok(Self { face, lod, x, y })
    }

    /// Returns the six LOD-0 root tiles covering the entire globe.
    #[must_use]
    pub const fn roots() -> [Self; 6] {
        [
            Self {
                face: CubeFace::PosX,
                lod: 0,
                x: 0,
                y: 0,
            },
            Self {
                face: CubeFace::NegX,
                lod: 0,
                x: 0,
                y: 0,
            },
            Self {
                face: CubeFace::PosY,
                lod: 0,
                x: 0,
                y: 0,
            },
            Self {
                face: CubeFace::NegY,
                lod: 0,
                x: 0,
                y: 0,
            },
            Self {
                face: CubeFace::PosZ,
                lod: 0,
                x: 0,
                y: 0,
            },
            Self {
                face: CubeFace::NegZ,
                lod: 0,
                x: 0,
                y: 0,
            },
        ]
    }

    /// Number of tiles per face edge at this tile's `lod`.
    #[must_use]
    pub const fn tiles_per_side(self) -> u32 {
        1_u32 << self.lod
    }

    /// Parent tile at `lod - 1`, or `None` if `lod == 0`.
    #[must_use]
    pub const fn parent(self) -> Option<Self> {
        if self.lod == 0 {
            None
        } else {
            Some(Self {
                face: self.face,
                lod: self.lod - 1,
                x: self.x >> 1,
                y: self.y >> 1,
            })
        }
    }

    /// Four child tiles at `lod + 1` in `(BL, BR, TL, TR)` order, or `None` at `MAX_SUPPORTED_LOD`.
    #[must_use]
    pub const fn children(self) -> Option<[Self; 4]> {
        if self.lod >= MAX_SUPPORTED_LOD {
            None
        } else {
            let child_lod = self.lod + 1;
            let cx = self.x << 1;
            let cy = self.y << 1;
            Some([
                Self {
                    face: self.face,
                    lod: child_lod,
                    x: cx,
                    y: cy,
                },
                Self {
                    face: self.face,
                    lod: child_lod,
                    x: cx + 1,
                    y: cy,
                },
                Self {
                    face: self.face,
                    lod: child_lod,
                    x: cx,
                    y: cy + 1,
                },
                Self {
                    face: self.face,
                    lod: child_lod,
                    x: cx + 1,
                    y: cy + 1,
                },
            ])
        }
    }

    /// Converts tile-local [`TileUv`] into face-space [`FaceUv`].
    #[must_use]
    pub fn tile_uv_to_face_uv(self, uv: TileUv) -> FaceUv {
        let inv_n = 1.0 / f64::from(self.tiles_per_side());
        FaceUv::clamped(
            (f64::from(self.x) + uv.u) * inv_n,
            (f64::from(self.y) + uv.v) * inv_n,
        )
    }

    /// Converts [`TileUv`] on this tile directly to authoritative [`EcefMeters`] on `planet`.
    #[must_use]
    pub fn tile_uv_to_ecef(self, planet: PlanetSpec, uv: TileUv, elevation_m: f64) -> EcefMeters {
        let face_uv = self.tile_uv_to_face_uv(uv);
        let unit = face_uv_to_unit(self.face, face_uv);
        unit.scale(planet.radius_meters() + elevation_m)
    }

    /// Finds the exact same-LOD neighbor across `edge`, handling both intra-face steps
    /// and 2-face cube-edge transitions via 3D face basis projection.
    #[must_use]
    pub fn neighbor(self, edge: TileEdge) -> TileNeighbor {
        let dim = self.tiles_per_side();
        match edge {
            TileEdge::Bottom if self.y > 0 => {
                return TileNeighbor {
                    tile: Self {
                        y: self.y - 1,
                        ..self
                    },
                    shared_edge_on_neighbor: TileEdge::Top,
                    crossed_face: false,
                };
            }
            TileEdge::Right if self.x + 1 < dim => {
                return TileNeighbor {
                    tile: Self {
                        x: self.x + 1,
                        ..self
                    },
                    shared_edge_on_neighbor: TileEdge::Left,
                    crossed_face: false,
                };
            }
            TileEdge::Top if self.y + 1 < dim => {
                return TileNeighbor {
                    tile: Self {
                        y: self.y + 1,
                        ..self
                    },
                    shared_edge_on_neighbor: TileEdge::Bottom,
                    crossed_face: false,
                };
            }
            TileEdge::Left if self.x > 0 => {
                return TileNeighbor {
                    tile: Self {
                        x: self.x - 1,
                        ..self
                    },
                    shared_edge_on_neighbor: TileEdge::Right,
                    crossed_face: false,
                };
            }
            _ => {}
        }

        let basis = self.face.basis();
        let inv_n = 1.0 / f64::from(dim);
        let (u_mid, v_mid, out_dir) = match edge {
            TileEdge::Bottom => ((f64::from(self.x) + 0.5) * inv_n, 0.0, basis.up.scale(-1.0)),
            TileEdge::Right => (1.0, (f64::from(self.y) + 0.5) * inv_n, basis.right),
            TileEdge::Top => ((f64::from(self.x) + 0.5) * inv_n, 1.0, basis.up),
            TileEdge::Left => (
                0.0,
                (f64::from(self.y) + 0.5) * inv_n,
                basis.right.scale(-1.0),
            ),
        };
        let s_mid = 2.0 * u_mid - 1.0;
        let t_mid = 2.0 * v_mid - 1.0;
        let edge_cube_pt = basis.normal + basis.right.scale(s_mid) + basis.up.scale(t_mid);

        let next_face = CubeFace::from_dominant_axis(out_dir);
        let next_basis = next_face.basis();
        let next_s = edge_cube_pt.dot(next_basis.right);
        let next_t = edge_cube_pt.dot(next_basis.up);
        let next_u = (0.5 * (next_s + 1.0)).clamp(0.0, 1.0);
        let next_v = (0.5 * (next_t + 1.0)).clamp(0.0, 1.0);

        let (nx, ny) = face_uv_to_tile_indices(self.lod, FaceUv::clamped(next_u, next_v));
        let back_s = basis.normal.dot(next_basis.right);
        let back_t = basis.normal.dot(next_basis.up);
        let shared_edge_on_neighbor = if back_s < -0.5 {
            TileEdge::Left
        } else if back_s > 0.5 {
            TileEdge::Right
        } else if back_t < -0.5 {
            TileEdge::Bottom
        } else {
            TileEdge::Top
        };

        TileNeighbor {
            tile: Self {
                face: next_face,
                lod: self.lod,
                x: nx,
                y: ny,
            },
            shared_edge_on_neighbor,
            crossed_face: true,
        }
    }
}

/// Maps an authoritative [`EcefMeters`] point to `(TileId, TileUv, elevation_m)` at `lod`.
///
/// # Errors
///
/// Returns [`GlobeError`] if `lod > MAX_SUPPORTED_LOD` or `ecef` is non-finite/zero.
pub fn ecef_to_tile_uv(
    planet: PlanetSpec,
    ecef: EcefMeters,
    lod: u8,
) -> Result<(TileId, TileUv, f64), GlobeError> {
    if lod > MAX_SUPPORTED_LOD {
        return Err(GlobeError::InvalidMaxLod {
            max_lod: lod,
            supported_max: MAX_SUPPORTED_LOD,
        });
    }
    let elevation_m = ecef.elevation_above(planet)?;
    let (face, face_uv) = ecef_to_face_uv(ecef)?;
    let (x, y, tile_uv) = face_uv_to_tile_uv(lod, face_uv);
    Ok((TileId { face, lod, x, y }, tile_uv, elevation_m))
}

/// Converts a [`FaceUv`] into `(tile_x, tile_y, TileUv)` at `lod`.
#[must_use]
#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "gx/gy are clamped to [0, dim - 1] where dim = 1 << lod <= 2^22"
)]
pub fn face_uv_to_tile_uv(lod: u8, face_uv: FaceUv) -> (u32, u32, TileUv) {
    let clamped_lod = lod.min(MAX_SUPPORTED_LOD);
    let dim = 1_u32 << clamped_lod;
    let dim_f = f64::from(dim);
    let gx = face_uv.u.clamp(0.0, 1.0) * dim_f;
    let gy = face_uv.v.clamp(0.0, 1.0) * dim_f;
    let tx = (gx.floor() as u32).min(dim - 1);
    let ty = (gy.floor() as u32).min(dim - 1);
    let tu = (gx - f64::from(tx)).clamp(0.0, 1.0);
    let tv = (gy - f64::from(ty)).clamp(0.0, 1.0);
    (tx, ty, TileUv::clamped(tu, tv))
}

#[inline]
#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "indices are clamped to [0, dim - 1] where dim <= 2^22"
)]
fn face_uv_to_tile_indices(lod: u8, face_uv: FaceUv) -> (u32, u32) {
    let dim = 1_u32 << lod.min(MAX_SUPPORTED_LOD);
    let dim_f = f64::from(dim);
    let tx = ((face_uv.u.clamp(0.0, 1.0) * dim_f).floor() as u32).min(dim - 1);
    let ty = ((face_uv.v.clamp(0.0, 1.0) * dim_f).floor() as u32).min(dim - 1);
    (tx, ty)
}
