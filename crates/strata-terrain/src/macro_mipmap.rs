use std::f64::consts::FRAC_PI_2;

use strata_globe::{CubeFace, ElevationRange, FaceUv, face_uv_to_unit};

use crate::error::TerrainError;
use crate::hydrology_index::HydrologyIndex;
use crate::macro_cache::{MICRO_MAX_BOUND_M, MacroCell};

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
    lod_depth: u8,
    vertices: &[MacroCell],
    level_offsets: &[usize],
    total_mip_per_face: usize,
    hydrology: &HydrologyIndex,
) -> Result<(Vec<MinMaxNode>, ElevationRange), TerrainError> {
    let n = resolution as usize;
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
        let f_vert_base = face.index() * face_vert_count;
        let f_mip_base = face.index() * total_mip_per_face;
        let leaf_offset = f_mip_base + level_offsets[usize::from(lod_depth)];

        for cy in 0..n {
            for cx in 0..n {
                let corners = [
                    vertices[f_vert_base + cy * stride + cx],
                    vertices[f_vert_base + cy * stride + cx + 1],
                    vertices[f_vert_base + (cy + 1) * stride + cx],
                    vertices[f_vert_base + (cy + 1) * stride + cx + 1],
                ];
                let (mut cell_min, mut cell_max) = corners_bounds(corners);

                #[expect(clippy::cast_precision_loss, reason = "cx, cy <= 512, exact in f64")]
                let center_uv =
                    FaceUv::clamped(((cx as f64) + 0.5) * inv_n, ((cy as f64) + 0.5) * inv_n);
                let cell_center = face_uv_to_unit(face, center_uv);
                if let Some((h_min, h_max)) = hydrology.cell_bounds_contribution(
                    face,
                    (cx >> bucket_shift).min(15),
                    (cy >> bucket_shift).min(15),
                    cell_center,
                    cell_half_diag_rad,
                ) {
                    cell_min = cell_min.min(h_min - 0.1);
                    cell_max = cell_max.max(h_max + 0.1);
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
fn corners_bounds(corners: [MacroCell; 4]) -> (f64, f64) {
    let mut macro_min = corners[0].macro_elevation_m;
    let mut macro_max = corners[0].macro_elevation_m;
    let mut r_max = corners[0].roughness;
    for c in &corners[1..] {
        macro_min = macro_min.min(c.macro_elevation_m);
        macro_max = macro_max.max(c.macro_elevation_m);
        r_max = r_max.max(c.roughness);
    }
    let margin = f64::from(r_max) * MICRO_MAX_BOUND_M + 0.1;
    (f64::from(macro_min) - margin, f64::from(macro_max) + margin)
}
