use bevy::prelude::*;
use strata_globe::{EcefMeters, LocalOrigin, PlanetSpec, TileId};
use strata_terrain::{TileVertexBase, WaterBodyKind, WaterSample, surface_albedo_rgb};

use crate::tile_mesh_build::TileColoredMesh;

pub(crate) fn compute_terrain_colors(
    unit_dirs: &[EcefMeters],
    normals: &[[f32; 3]],
    base_samples: &[TileVertexBase],
) -> (Vec<[f32; 4]>, bool, [f32; 3]) {
    let mut colors = Vec::with_capacity(base_samples.len());
    let mut has_water = false;
    let mut shore_hint_rgb = [0.08_f32, 0.36, 0.48];

    for idx in 0..base_samples.len() {
        let unit_dir = unit_dirs[idx];
        let normal = normals[idx];
        let base = base_samples[idx];

        let dot = (f64::from(normal[0]) * unit_dir.x
            + f64::from(normal[1]) * unit_dir.y
            + f64::from(normal[2]) * unit_dir.z)
            .clamp(0.0, 1.0);
        let slope_sin = (1.0 - dot * dot).max(0.0).sqrt();

        let srgb = match base.water.kind {
            WaterBodyKind::Dry => surface_albedo_rgb(
                base.elevation_m,
                slope_sin,
                base.water,
                base.climate,
                base.biome,
            ),
            WaterBodyKind::Ocean => {
                has_water = true;
                shore_hint_rgb = [0.09, 0.42, 0.52];
                seabed_srgb(base.water.depth_m, slope_sin)
            }
            WaterBodyKind::River(_) => {
                has_water = true;
                shore_hint_rgb = [0.06, 0.29, 0.42];
                riverbed_srgb(base.water.depth_m)
            }
            WaterBodyKind::Lake(_) => {
                has_water = true;
                shore_hint_rgb = if base.water.is_freshwater {
                    [0.04, 0.24, 0.43]
                } else {
                    [0.06, 0.31, 0.38]
                };
                lakebed_srgb(base.water.depth_m)
            }
        };
        colors.push(srgb_to_linear_rgba(srgb));
    }

    (colors, has_water, shore_hint_rgb)
}

#[expect(
    clippy::cast_possible_truncation,
    reason = "water surface normals intentionally narrow to f32 vertex attributes"
)]
pub(crate) fn build_water_stage(
    planet: PlanetSpec,
    tile: TileId,
    tile_origin: LocalOrigin,
    unit_dirs: &[EcefMeters],
    base_samples: &[TileVertexBase],
    uvs: Vec<[f32; 2]>,
    shore_hint_rgb: [f32; 3],
) -> TileColoredMesh {
    let r_base = planet.radius_meters();
    let min_r = 1.0_f64.min(r_base * 0.5);
    let cell_arc = planet.cell_arc_length_meters(tile.lod);
    let tuck_m = (cell_arc * 0.08).clamp(0.5, 45.0);
    let vertex_count = unit_dirs.len();

    let mut positions = Vec::with_capacity(vertex_count);
    let mut normals = Vec::with_capacity(vertex_count);
    let mut colors = Vec::with_capacity(vertex_count);

    for idx in 0..vertex_count {
        let unit_dir = unit_dirs[idx];
        let base = base_samples[idx];
        let (ecef, rgb) = if base.water.kind == WaterBodyKind::Dry {
            let elev = if base.elevation_m.is_finite() {
                base.elevation_m.max(min_r - r_base)
            } else {
                0.0
            };
            let terrain_r = (r_base + elev).max(min_r);
            let tucked_r = (terrain_r - tuck_m).max(min_r);
            (unit_dir.scale(tucked_r), shore_hint_rgb)
        } else {
            let surf_r = (r_base + base.water.surface_elevation_m).max(min_r);
            (unit_dir.scale(surf_r), water_surface_srgb(base.water))
        };
        positions.push(tile_origin.to_render_vec3(ecef).to_array());
        normals.push([unit_dir.x as f32, unit_dir.y as f32, unit_dir.z as f32]);
        colors.push(srgb_to_linear_rgba(rgb));
    }

    TileColoredMesh {
        positions,
        normals,
        uvs,
        colors,
    }
}

#[inline]
fn srgb_to_linear_rgba(rgb: [f32; 3]) -> [f32; 4] {
    Color::srgb(rgb[0], rgb[1], rgb[2])
        .to_linear()
        .to_f32_array()
}

#[expect(
    clippy::cast_possible_truncation,
    reason = "normalized depth blend factors narrow cleanly to f32"
)]
fn water_surface_srgb(water: WaterSample) -> [f32; 3] {
    match water.kind {
        WaterBodyKind::Ocean => {
            let d = water.depth_m.max(0.0);
            if d < 12.0 {
                let t = (d / 12.0).clamp(0.0, 1.0) as f32;
                lerp_rgb([0.11, 0.48, 0.56], [0.08, 0.39, 0.51], t)
            } else if d < 200.0 {
                let t = ((d - 12.0) / 188.0).clamp(0.0, 1.0) as f32;
                lerp_rgb([0.08, 0.39, 0.51], [0.03, 0.20, 0.38], t)
            } else {
                let t = ((d - 200.0) / 4_000.0).clamp(0.0, 1.0) as f32;
                lerp_rgb([0.03, 0.20, 0.38], [0.012, 0.065, 0.17], t)
            }
        }
        WaterBodyKind::River(_) => [0.06, 0.29, 0.42],
        WaterBodyKind::Lake(_) => {
            if water.is_freshwater {
                [0.04, 0.24, 0.43]
            } else {
                [0.06, 0.31, 0.38]
            }
        }
        WaterBodyKind::Dry => [0.07, 0.34, 0.46],
    }
}

#[expect(
    clippy::cast_possible_truncation,
    reason = "normalized bathymetric blend factors narrow cleanly to f32"
)]
fn seabed_srgb(depth_m: f64, slope_sin: f64) -> [f32; 3] {
    let d = depth_m.max(0.0);
    let mut rgb = if d < 15.0 {
        let t = (d / 15.0).clamp(0.0, 1.0) as f32;
        lerp_rgb([0.52, 0.48, 0.37], [0.36, 0.36, 0.32], t)
    } else if d < 200.0 {
        let t = ((d - 15.0) / 185.0).clamp(0.0, 1.0) as f32;
        lerp_rgb([0.36, 0.36, 0.32], [0.21, 0.24, 0.27], t)
    } else {
        let t = ((d - 200.0) / 4_000.0).clamp(0.0, 1.0) as f32;
        lerp_rgb([0.21, 0.24, 0.27], [0.09, 0.11, 0.15], t)
    };
    let slope_rock = ((slope_sin - 0.18) / 0.45).clamp(0.0, 1.0) as f32;
    if slope_rock > 0.0 {
        rgb = lerp_rgb(rgb, [0.18, 0.19, 0.21], slope_rock * 0.55);
    }
    rgb
}

#[expect(
    clippy::cast_possible_truncation,
    reason = "normalized riverbed depth factor narrows cleanly to f32"
)]
fn riverbed_srgb(depth_m: f64) -> [f32; 3] {
    let t = (depth_m.max(0.0) / 40.0).clamp(0.0, 1.0) as f32;
    lerp_rgb([0.38, 0.34, 0.27], [0.22, 0.21, 0.19], t)
}

#[expect(
    clippy::cast_possible_truncation,
    reason = "normalized lakebed depth factor narrows cleanly to f32"
)]
fn lakebed_srgb(depth_m: f64) -> [f32; 3] {
    let t = (depth_m.max(0.0) / 400.0).clamp(0.0, 1.0) as f32;
    lerp_rgb([0.34, 0.34, 0.29], [0.15, 0.17, 0.19], t)
}

#[inline]
fn lerp_rgb(a: [f32; 3], b: [f32; 3], t: f32) -> [f32; 3] {
    [
        a[0] + (b[0] - a[0]) * t,
        a[1] + (b[1] - a[1]) * t,
        a[2] + (b[2] - a[2]) * t,
    ]
}
