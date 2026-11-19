use crate::coords::{EcefMeters, LocalOrigin};
use crate::cube::face_uv_to_unit;
use crate::elevation::{ElevationSampler, FlatElevation};
use crate::lod::CoarserEdges;
use crate::planet::PlanetSpec;
use crate::tile::{TileId, TileUv};

/// Authoritative `f64` tile mesh prior to camera-relative `LocalOrigin` narrowing.
#[derive(Debug, Clone, PartialEq)]
pub struct TileMeshF64 {
    /// Tile identifier.
    pub tile: TileId,
    /// Edge stitching flags matching adjacent `LOD - 1` neighbors.
    pub coarser_edges: CoarserEdges,
    /// Grid segments per edge (`N`). Vertex grid is `(N + 1) x (N + 1)`.
    pub grid_segments: u32,
    /// Vertex positions in `f64` ECEF meters.
    pub positions_ecef: Vec<EcefMeters>,
    /// Outward unit surface normals (`f32`).
    pub normals: Vec<[f32; 3]>,
    /// Tile-local UV coordinates `[0, 1]^2`.
    pub uvs: Vec<[f32; 2]>,
    /// Watertight triangle indices (`CCW` outward winding).
    pub indices: Vec<u32>,
}

/// Camera-relative `f32` render mesh produced from a [`TileMeshF64`] and [`LocalOrigin`].
#[derive(Debug, Clone, PartialEq)]
pub struct TileRenderMesh {
    /// Tile identifier.
    pub tile: TileId,
    /// Edge stitching flags matching adjacent `LOD - 1` neighbors.
    pub coarser_edges: CoarserEdges,
    /// Origin used when narrowing vertex positions to `f32`.
    pub origin: LocalOrigin,
    /// Vertex positions relative to `origin` in `f32` meters.
    pub positions: Vec<[f32; 3]>,
    /// Outward unit surface normals (`f32`).
    pub normals: Vec<[f32; 3]>,
    /// Tile-local UV coordinates `[0, 1]^2`.
    pub uvs: Vec<[f32; 2]>,
    /// Watertight triangle indices (`CCW` outward winding).
    pub indices: Vec<u32>,
}

impl TileMeshF64 {
    /// Converts this authoritative `f64` mesh into a [`TileRenderMesh`] relative to `origin`.
    #[must_use]
    pub fn to_render_mesh(&self, origin: LocalOrigin) -> TileRenderMesh {
        let positions = self
            .positions_ecef
            .iter()
            .map(|&p| origin.to_render_vec3(p).to_array())
            .collect();
        TileRenderMesh {
            tile: self.tile,
            coarser_edges: self.coarser_edges,
            origin,
            positions,
            normals: self.normals.clone(),
            uvs: self.uvs.clone(),
            indices: self.indices.clone(),
        }
    }
}

/// Builds an authoritative `f64` smooth-sphere patch mesh for `tile` with watertight
/// `coarser_edges` edge stitching.
#[must_use]
pub fn build_tile_mesh_f64(
    planet: PlanetSpec,
    tile: TileId,
    coarser_edges: CoarserEdges,
) -> TileMeshF64 {
    build_tile_mesh_f64_with_elevation(planet, tile, coarser_edges, &FlatElevation)
}

/// Builds an authoritative `f64` terrain-displaced tile mesh using `sampler`, keeping
/// shared vertices bit-identical across intra-face seams, cube edges, and 3-face corners.
#[must_use]
#[expect(
    clippy::cast_possible_truncation,
    reason = "surface unit normals and [0,1] UVs intentionally narrow to f32 for vertex attributes"
)]
pub fn build_tile_mesh_f64_with_elevation(
    planet: PlanetSpec,
    tile: TileId,
    coarser_edges: CoarserEdges,
    sampler: &dyn ElevationSampler,
) -> TileMeshF64 {
    let n = planet.grid_segments();
    let stride = (n + 1) as usize;
    let vertex_count = stride * stride;

    let mut positions_ecef = Vec::with_capacity(vertex_count);
    let mut normals = Vec::with_capacity(vertex_count);
    let mut uvs = Vec::with_capacity(vertex_count);

    let inv_n = 1.0 / f64::from(n);
    let r_base = planet.radius_meters();
    let min_r = 1.0_f64.min(r_base * 0.5);
    let is_flat = sampler.global_range().is_flat();
    let d_theta = (planet.cell_arc_length_meters(tile.lod) / r_base).clamp(1e-7, 0.05);
    let (sin_d, cos_d) = d_theta.sin_cos();

    for iy in 0..=n {
        let v = f64::from(iy) * inv_n;
        for ix in 0..=n {
            let u = f64::from(ix) * inv_n;
            let face_uv = tile.tile_uv_to_face_uv(TileUv::clamped(u, v));
            let unit_dir = face_uv_to_unit(tile.face, face_uv);

            let (ecef, normal) = if is_flat {
                (unit_dir.scale(r_base), unit_dir)
            } else {
                let radius = sample_clamped_radius(r_base, min_r, unit_dir, sampler);
                let normal =
                    terrain_normal_canonical(r_base, min_r, unit_dir, sin_d, cos_d, sampler);
                (unit_dir.scale(radius), normal)
            };

            positions_ecef.push(ecef);
            normals.push([normal.x as f32, normal.y as f32, normal.z as f32]);
            uvs.push([u as f32, v as f32]);
        }
    }

    let indices = build_stitched_indices(n, coarser_edges);

    TileMeshF64 {
        tile,
        coarser_edges,
        grid_segments: n,
        positions_ecef,
        normals,
        uvs,
        indices,
    }
}

/// Convenience helper building a smooth-sphere [`TileRenderMesh`] directly relative to `origin`.
#[must_use]
pub fn build_tile_render_mesh(
    planet: PlanetSpec,
    tile: TileId,
    coarser_edges: CoarserEdges,
    origin: LocalOrigin,
) -> TileRenderMesh {
    build_tile_render_mesh_with_elevation(planet, tile, coarser_edges, origin, &FlatElevation)
}

/// Convenience helper building a terrain-displaced [`TileRenderMesh`] relative to `origin`.
#[must_use]
pub fn build_tile_render_mesh_with_elevation(
    planet: PlanetSpec,
    tile: TileId,
    coarser_edges: CoarserEdges,
    origin: LocalOrigin,
    sampler: &dyn ElevationSampler,
) -> TileRenderMesh {
    build_tile_mesh_f64_with_elevation(planet, tile, coarser_edges, sampler).to_render_mesh(origin)
}

#[inline]
fn sample_clamped_radius(
    r_base: f64,
    min_r: f64,
    unit_dir: EcefMeters,
    sampler: &dyn ElevationSampler,
) -> f64 {
    let elev = sampler.sample_elevation(unit_dir);
    if elev.is_finite() {
        (r_base + elev).max(min_r)
    } else {
        r_base.max(min_r)
    }
}

/// Computes a smooth outward terrain normal using central differences in a canonical
/// tangent plane depending solely on `unit_dir` and angular cell scale `(sin_d, cos_d)`.
fn terrain_normal_canonical(
    r_base: f64,
    min_r: f64,
    unit_dir: EcefMeters,
    sin_d: f64,
    cos_d: f64,
    sampler: &dyn ElevationSampler,
) -> EcefMeters {
    let (t1, t2) = canonical_tangent_basis(unit_dir);
    let u1_pos = (unit_dir.scale(cos_d) + t1.scale(sin_d))
        .normalized()
        .unwrap_or(unit_dir);
    let u1_neg = (unit_dir.scale(cos_d) - t1.scale(sin_d))
        .normalized()
        .unwrap_or(unit_dir);
    let u2_pos = (unit_dir.scale(cos_d) + t2.scale(sin_d))
        .normalized()
        .unwrap_or(unit_dir);
    let u2_neg = (unit_dir.scale(cos_d) - t2.scale(sin_d))
        .normalized()
        .unwrap_or(unit_dir);

    let p1_pos = u1_pos.scale(sample_clamped_radius(r_base, min_r, u1_pos, sampler));
    let p1_neg = u1_neg.scale(sample_clamped_radius(r_base, min_r, u1_neg, sampler));
    let p2_pos = u2_pos.scale(sample_clamped_radius(r_base, min_r, u2_pos, sampler));
    let p2_neg = u2_neg.scale(sample_clamped_radius(r_base, min_r, u2_neg, sampler));

    let dp1 = p1_pos - p1_neg;
    let dp2 = p2_pos - p2_neg;
    let Ok(normal) = dp1.cross(dp2).normalized() else {
        return unit_dir;
    };
    if normal.dot(unit_dir) > 0.0 {
        normal
    } else {
        unit_dir
    }
}

/// Deterministic right-handed tangent basis `(t1, t2)` with `t1 x t2 == n`,
/// independent of cube-face orientation so shared seam/corner vertices match.
fn canonical_tangent_basis(n: EcefMeters) -> (EcefMeters, EcefMeters) {
    let ax = n.x.abs();
    let ay = n.y.abs();
    let az = n.z.abs();
    let ref_axis = if ax <= ay && ax <= az {
        EcefMeters::from_raw(1.0, 0.0, 0.0)
    } else if ay <= az {
        EcefMeters::from_raw(0.0, 1.0, 0.0)
    } else {
        EcefMeters::from_raw(0.0, 0.0, 1.0)
    };
    let t1 = ref_axis
        .cross(n)
        .normalized()
        .unwrap_or(EcefMeters::from_raw(1.0, 0.0, 0.0));
    let t2 = n
        .cross(t1)
        .normalized()
        .unwrap_or(EcefMeters::from_raw(0.0, 1.0, 0.0));
    (t1, t2)
}

/// Generates a watertight, crack-free triangle index buffer for an `n x n` quad grid
/// (even `n >= 4`) where any subset of outer edges may drop odd boundary vertices
/// to match an adjacent `LOD - 1` tile bit-for-bit.
#[must_use]
pub fn build_stitched_indices(n: u32, coarser: CoarserEdges) -> Vec<u32> {
    let stride = n + 1;
    let idx = |x: u32, y: u32| -> u32 { y * stride + x };
    let mut indices = Vec::with_capacity((n * n * 6) as usize);
    let mut push_tri = |a: (u32, u32), b: (u32, u32), c: (u32, u32)| {
        indices.push(idx(a.0, a.1));
        indices.push(idx(b.0, b.1));
        indices.push(idx(c.0, c.1));
    };

    // 1. Interior cells: x in 1..n-1, y in 1..n-1.
    for cy in 1..(n - 1) {
        for cx in 1..(n - 1) {
            push_tri((cx, cy), (cx + 1, cy), (cx + 1, cy + 1));
            push_tri((cx, cy), (cx + 1, cy + 1), (cx, cy + 1));
        }
    }

    // 2. Bottom strip (y = 0..1, x = 0..n), owns all four bottom-row corners' horizontal halves.
    let half = n / 2;
    for k in 0..half {
        let x0 = 2 * k;
        let x1 = x0 + 1;
        let x2 = x0 + 2;
        let skip_left_corner = k == 0 && coarser.left();
        let skip_right_corner = k == half - 1 && coarser.right();
        if coarser.bottom() {
            push_tri((x0, 0), (x2, 0), (x1, 1));
            if !skip_left_corner {
                push_tri((x0, 0), (x1, 1), (x0, 1));
            }
            if !skip_right_corner {
                push_tri((x2, 0), (x2, 1), (x1, 1));
            }
        } else {
            push_tri((x0, 0), (x1, 0), (x1, 1));
            if !skip_left_corner {
                push_tri((x0, 0), (x1, 1), (x0, 1));
            }
            push_tri((x1, 0), (x2, 0), (x1, 1));
            if !skip_right_corner {
                push_tri((x2, 0), (x2, 1), (x1, 1));
            }
        }
    }

    // 3. Top strip (y = n-1..n, x = 0..n), owns both top-row corners' horizontal halves.
    for k in 0..half {
        let x0 = 2 * k;
        let x1 = x0 + 1;
        let x2 = x0 + 2;
        let skip_left_corner = k == 0 && coarser.left();
        let skip_right_corner = k == half - 1 && coarser.right();
        if coarser.top() {
            push_tri((x2, n), (x0, n), (x1, n - 1));
            if !skip_left_corner {
                push_tri((x0, n), (x0, n - 1), (x1, n - 1));
            }
            if !skip_right_corner {
                push_tri((x2, n), (x1, n - 1), (x2, n - 1));
            }
        } else {
            if !skip_left_corner {
                push_tri((x0, n), (x0, n - 1), (x1, n - 1));
            }
            push_tri((x1, n), (x0, n), (x1, n - 1));
            push_tri((x2, n), (x1, n), (x1, n - 1));
            if !skip_right_corner {
                push_tri((x2, n), (x1, n - 1), (x2, n - 1));
            }
        }
    }

    // 4. Left strip (x = 0..1).
    emit_vertical_strip(n, true, coarser.left(), &mut push_tri);

    // 5. Right strip (x = n-1..n).
    emit_vertical_strip(n, false, coarser.right(), &mut push_tri);

    indices
}

fn emit_vertical_strip<F>(n: u32, is_left: bool, is_coarser: bool, push_tri: &mut F)
where
    F: FnMut((u32, u32), (u32, u32), (u32, u32)),
{
    let half = n / 2;
    if is_coarser {
        for k in 0..half {
            let y0 = 2 * k;
            let y1 = y0 + 1;
            let y2 = y0 + 2;
            let is_bottom_corner = k == 0;
            let is_top_corner = k == half - 1;
            if is_left {
                push_tri((0, y2), (0, y0), (1, y1));
                if !is_bottom_corner {
                    push_tri((0, y0), (1, y0), (1, y1));
                }
                if !is_top_corner {
                    push_tri((0, y2), (1, y1), (1, y2));
                }
            } else {
                push_tri((n, y0), (n, y2), (n - 1, y1));
                if !is_bottom_corner {
                    push_tri((n, y0), (n - 1, y1), (n - 1, y0));
                }
                if !is_top_corner {
                    push_tri((n, y2), (n - 1, y2), (n - 1, y1));
                }
            }
        }
    } else {
        for cy in 1..(n - 1) {
            if is_left {
                push_tri((0, cy), (1, cy), (1, cy + 1));
                push_tri((0, cy), (1, cy + 1), (0, cy + 1));
            } else {
                push_tri((n - 1, cy), (n, cy), (n, cy + 1));
                push_tri((n - 1, cy), (n, cy + 1), (n - 1, cy + 1));
            }
        }
    }
}
