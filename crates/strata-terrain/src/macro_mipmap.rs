use std::f64::consts::{FRAC_PI_2, PI, TAU};
use std::sync::OnceLock;

use strata_globe::{
    CubeFace, EcefMeters, ElevationRange, FaceUv, ecef_to_face_uv_unit, face_uv_to_unit,
};

use crate::earth_geography::ElevationGridSource;
use crate::error::TerrainError;
use crate::hydrology_index::HydrologyIndex;
use crate::macro_cache::{DEFAULT_MACRO_RESOLUTION, MICRO_MAX_BOUND_M, MacroCell};

static EARTH_DEFAULT_LEAF_BOUNDS: OnceLock<(Vec<f64>, Vec<f64>)> = OnceLock::new();

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct MinMaxNode {
    pub min_m: f32,
    pub max_m: f32,
}

pub(crate) fn compute_mip_layout(lod_depth: u8) -> (Vec<usize>, usize) {
    let mut level_offsets = Vec::with_capacity(usize::from(lod_depth) + 1);
    let mut total_mip_per_face = 0_usize;
    for lod in 0..=lod_depth {
        level_offsets.push(total_mip_per_face);
        let dim = 1_usize << lod;
        total_mip_per_face += dim * dim;
    }
    (level_offsets, total_mip_per_face)
}

pub(crate) fn build_mip_pyramid(
    resolution: u32,
    vertices: &[MacroCell],
    vertex_bounds: &[(f64, f64)],
    elevation_source: &ElevationGridSource,
    level_offsets: &[usize],
    total_mip_per_face: usize,
    hydrology: &HydrologyIndex,
) -> Result<(Vec<MinMaxNode>, ElevationRange), TerrainError> {
    let lod_depth = u8::try_from(resolution.trailing_zeros()).unwrap_or(7);
    let n = resolution as usize;
    let face_cells = n * n;
    let computed_bounds;
    let (leaf_min, leaf_max): (&[f64], &[f64]) =
        if matches!(elevation_source, ElevationGridSource::Earth(_))
            && resolution == DEFAULT_MACRO_RESOLUTION
        {
            let (cached_min, cached_max) = EARTH_DEFAULT_LEAF_BOUNDS.get_or_init(|| {
                compute_leaf_dem_bounds(resolution, vertex_bounds, elevation_source)
            });
            (cached_min, cached_max)
        } else {
            computed_bounds = compute_leaf_dem_bounds(resolution, vertex_bounds, elevation_source);
            (&computed_bounds.0, &computed_bounds.1)
        };

    let stride = n + 1;
    let face_vert_count = stride * stride;
    let inv_n = 1.0 / f64::from(resolution);
    let cell_half_diag_rad = (FRAC_PI_2 * inv_n) * std::f64::consts::SQRT_2 * 0.65;
    let bucket_shift = lod_depth.saturating_sub(4);

    let mut mip_nodes = vec![
        MinMaxNode {
            min_m: 0.0,
            max_m: 0.0
        };
        6 * total_mip_per_face
    ];
    let mut global_min = f64::INFINITY;
    let mut global_max = f64::NEG_INFINITY;

    for face in CubeFace::ALL {
        let f_cell_base = face.index() * face_cells;
        let f_vert_base = face.index() * face_vert_count;
        let f_mip_base = face.index() * total_mip_per_face;
        let leaf_offset = f_mip_base + level_offsets[usize::from(lod_depth)];

        for cy in 0..n {
            let by = (cy >> bucket_shift).min(15);
            for cx in 0..n {
                let dem_min = leaf_min[f_cell_base + cy * n + cx];
                let dem_max = leaf_max[f_cell_base + cy * n + cx];
                let r_max = max_corner_roughness([
                    vertices[f_vert_base + cy * stride + cx],
                    vertices[f_vert_base + cy * stride + cx + 1],
                    vertices[f_vert_base + (cy + 1) * stride + cx],
                    vertices[f_vert_base + (cy + 1) * stride + cx + 1],
                ]);

                let (mut cell_min, mut cell_max) = if dem_max <= 0.0 {
                    (dem_min - 0.1, dem_max + 0.1)
                } else {
                    let margin = f64::from(r_max) * MICRO_MAX_BOUND_M + 0.1;
                    (dem_min - margin, dem_max + margin)
                };

                let bx = (cx >> bucket_shift).min(15);
                if hydrology.bucket_has_features(face, bx, by) {
                    #[expect(clippy::cast_precision_loss, reason = "cx, cy <= 512, exact in f64")]
                    let center_uv =
                        FaceUv::clamped(((cx as f64) + 0.5) * inv_n, ((cy as f64) + 0.5) * inv_n);
                    let cell_center = face_uv_to_unit(face, center_uv);
                    if let Some((h_min, h_max)) = hydrology.cell_bounds_contribution(
                        face,
                        bx,
                        by,
                        cell_center,
                        cell_half_diag_rad,
                    ) {
                        cell_min = cell_min.min(h_min - 0.1);
                        cell_max = cell_max.max(h_max + 0.1);
                    }
                }

                #[expect(
                    clippy::cast_possible_truncation,
                    reason = "conservative bounds intentionally narrow slightly with margin"
                )]
                {
                    mip_nodes[leaf_offset + cy * n + cx] = MinMaxNode {
                        min_m: (cell_min as f32) - 0.05,
                        max_m: (cell_max as f32) + 0.05,
                    };
                }
            }
        }

        downsample_face_pyramid(&mut mip_nodes, f_mip_base, lod_depth, level_offsets);

        let root = mip_nodes[f_mip_base];
        global_min = global_min.min(f64::from(root.min_m));
        global_max = global_max.max(f64::from(root.max_m));
    }

    Ok((mip_nodes, ElevationRange::new(global_min, global_max)?))
}

fn compute_leaf_dem_bounds(
    resolution: u32,
    vertex_bounds: &[(f64, f64)],
    elevation_source: &ElevationGridSource,
) -> (Vec<f64>, Vec<f64>) {
    let n = resolution as usize;
    let face_cells = n * n;
    let mut leaf_min = vec![f64::INFINITY; 6 * face_cells];
    let mut leaf_max = vec![f64::NEG_INFINITY; 6 * face_cells];

    seed_leaf_bounds_from_vertices(resolution, vertex_bounds, &mut leaf_min, &mut leaf_max);
    seed_face_edge_midpoints(resolution, elevation_source, &mut leaf_min, &mut leaf_max);
    scatter_dem_texels_into_leaves(resolution, elevation_source, &mut leaf_min, &mut leaf_max);
    (leaf_min, leaf_max)
}

fn seed_leaf_bounds_from_vertices(
    resolution: u32,
    vertex_bounds: &[(f64, f64)],
    leaf_min: &mut [f64],
    leaf_max: &mut [f64],
) {
    let n = resolution as usize;
    let stride = n + 1;
    let face_verts = stride * stride;
    let face_cells = n * n;

    for face in CubeFace::ALL {
        let f_vert_base = face.index() * face_verts;
        let f_cell_base = face.index() * face_cells;
        for cy in 0..n {
            for cx in 0..n {
                let (min00, max00) = vertex_bounds[f_vert_base + cy * stride + cx];
                let (min10, max10) = vertex_bounds[f_vert_base + cy * stride + cx + 1];
                let (min01, max01) = vertex_bounds[f_vert_base + (cy + 1) * stride + cx];
                let (min11, max11) = vertex_bounds[f_vert_base + (cy + 1) * stride + cx + 1];
                let idx = f_cell_base + cy * n + cx;
                leaf_min[idx] = min00.min(min10).min(min01).min(min11);
                leaf_max[idx] = max00.max(max10).max(max01).max(max11);
            }
        }
    }
}

fn seed_face_edge_midpoints(
    resolution: u32,
    elevation_source: &ElevationGridSource,
    leaf_min: &mut [f64],
    leaf_max: &mut [f64],
) {
    let n = resolution as usize;
    let inv_n = 1.0 / f64::from(resolution);
    let face_cells = n * n;

    for face in CubeFace::ALL {
        let f_cell_base = face.index() * face_cells;
        for i in 0..n {
            #[expect(clippy::cast_precision_loss, reason = "i <= 512, exact in f64")]
            let mid = ((i as f64) + 0.5) * inv_n;

            let edges = [
                (i, 0, mid, 0.0),
                (i, n - 1, mid, 1.0),
                (0, i, 0.0, mid),
                (n - 1, i, 1.0, mid),
            ];
            for (cx, cy, u, v) in edges {
                let unit = face_uv_to_unit(face, FaceUv::clamped(u, v));
                let (qmin, qmax) = elevation_source.sample_quad_minmax(unit);
                let idx = f_cell_base + cy * n + cx;
                leaf_min[idx] = leaf_min[idx].min(qmin);
                leaf_max[idx] = leaf_max[idx].max(qmax);
            }
        }
    }
}

#[expect(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "DEM grid dimensions and clamped face UVs stay strictly within usize bounds"
)]
fn scatter_dem_texels_into_leaves(
    resolution: u32,
    elevation_source: &ElevationGridSource,
    leaf_min: &mut [f64],
    leaf_max: &mut [f64],
) {
    let dim = resolution as usize;
    let res_f = f64::from(resolution);
    let face_cells = dim * dim;

    let (width, height, pixel_centered) = match elevation_source {
        ElevationGridSource::Earth(dem) => (dem.width(), dem.height(), true),
        ElevationGridSource::CustomDem(grid) => {
            if grid.width().saturating_mul(grid.height()) > 4_194_304 {
                return;
            }
            (grid.width(), grid.height(), false)
        }
    };

    let mut sin_lat = Vec::with_capacity(height);
    let mut cos_lat = Vec::with_capacity(height);
    for row in 0..height {
        let lat = if pixel_centered {
            FRAC_PI_2 - (((row as f64) + 0.5) / (height as f64)) * PI
        } else {
            FRAC_PI_2 - ((row as f64) / ((height - 1) as f64)) * PI
        };
        let (sin_val, cos_val) = lat.sin_cos();
        sin_lat.push(sin_val);
        cos_lat.push(cos_val);
    }

    let mut sin_lon = Vec::with_capacity(width);
    let mut cos_lon = Vec::with_capacity(width);
    for col in 0..width {
        let lon = if pixel_centered {
            -PI + (((col as f64) + 0.5) / (width as f64)) * TAU
        } else {
            -PI + ((col as f64) / (width as f64)) * TAU
        };
        let (sin_val, cos_val) = lon.sin_cos();
        sin_lon.push(sin_val);
        cos_lon.push(cos_val);
    }

    for row in 0..height {
        let sl = sin_lat[row];
        let cl = cos_lat[row];
        let row_off = row * width;
        for col in 0..width {
            let elev = match elevation_source {
                ElevationGridSource::Earth(dem) => f64::from(dem.elevations_m()[row_off + col]),
                ElevationGridSource::CustomDem(grid) => {
                    f64::from(grid.elevations_m()[row_off + col])
                }
            };
            let unit = EcefMeters::from_raw(cl * sin_lon[col], sl, cl * cos_lon[col]);
            let (face, uv) = ecef_to_face_uv_unit(unit);
            let gx = uv.u.clamp(0.0, 1.0) * res_f;
            let gy = uv.v.clamp(0.0, 1.0) * res_f;
            let cx = (gx.floor() as usize).min(dim - 1);
            let cy = (gy.floor() as usize).min(dim - 1);
            let fx = gx - (cx as f64);
            let fy = gy - (cy as f64);

            let nx = if fx < 0.45 && cx > 0 {
                cx - 1
            } else if fx > 0.55 && cx + 1 < dim {
                cx + 1
            } else {
                cx
            };
            let ny = if fy < 0.45 && cy > 0 {
                cy - 1
            } else if fy > 0.55 && cy + 1 < dim {
                cy + 1
            } else {
                cy
            };

            let f_base = face.index() * face_cells;
            let base_idx = f_base + cy * dim + cx;
            leaf_min[base_idx] = leaf_min[base_idx].min(elev);
            leaf_max[base_idx] = leaf_max[base_idx].max(elev);
            if nx != cx {
                let horiz_idx = f_base + cy * dim + nx;
                leaf_min[horiz_idx] = leaf_min[horiz_idx].min(elev);
                leaf_max[horiz_idx] = leaf_max[horiz_idx].max(elev);
            }
            if ny != cy {
                let vert_idx = f_base + ny * dim + cx;
                leaf_min[vert_idx] = leaf_min[vert_idx].min(elev);
                leaf_max[vert_idx] = leaf_max[vert_idx].max(elev);
                if nx != cx {
                    let diag_idx = f_base + ny * dim + nx;
                    leaf_min[diag_idx] = leaf_min[diag_idx].min(elev);
                    leaf_max[diag_idx] = leaf_max[diag_idx].max(elev);
                }
            }
        }
    }
}

fn downsample_face_pyramid(
    mip_nodes: &mut [MinMaxNode],
    f_mip_base: usize,
    lod_depth: u8,
    level_offsets: &[usize],
) {
    for lod in (0..lod_depth).rev() {
        let dst_dim = 1_usize << lod;
        let src_dim = dst_dim << 1;
        let dst_base = f_mip_base + level_offsets[usize::from(lod)];
        let src_base = f_mip_base + level_offsets[usize::from(lod + 1)];
        for y in 0..dst_dim {
            for x in 0..dst_dim {
                let sx = x << 1;
                let sy = y << 1;
                let n00 = mip_nodes[src_base + sy * src_dim + sx];
                let n10 = mip_nodes[src_base + sy * src_dim + sx + 1];
                let n01 = mip_nodes[src_base + (sy + 1) * src_dim + sx];
                let n11 = mip_nodes[src_base + (sy + 1) * src_dim + sx + 1];
                mip_nodes[dst_base + y * dst_dim + x] = MinMaxNode {
                    min_m: n00.min_m.min(n10.min_m).min(n01.min_m).min(n11.min_m),
                    max_m: n00.max_m.max(n10.max_m).max(n01.max_m).max(n11.max_m),
                };
            }
        }
    }
}

#[inline]
fn max_corner_roughness(corners: [MacroCell; 4]) -> f32 {
    corners[0]
        .roughness
        .max(corners[1].roughness)
        .max(corners[2].roughness)
        .max(corners[3].roughness)
}
