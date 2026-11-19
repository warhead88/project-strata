use bevy::asset::RenderAssetUsages;
use bevy::mesh::{Indices, PrimitiveTopology};
use bevy::prelude::*;
use strata_globe::{
    CoarserEdges, EcefMeters, ElevationSampler, LocalOrigin, PlanetSpec, TileId, TileUv,
    build_tile_mesh_f64_with_elevation, face_uv_to_unit,
};
use strata_terrain::{TerrainWorld, WaterBodyKind, WaterSample};

use crate::terrain::sample_clamped_elevation;

/// Single colored tile mesh (`terrain` or `water`) in tile-local `f32` coordinates
/// relative to [`BuiltTileMeshes::tile_origin`].
#[derive(Debug, Clone, PartialEq)]
pub struct TileColoredMesh {
    /// Vertex positions relative to `tile_origin` in `f32` meters.
    pub positions: Vec<[f32; 3]>,
    /// Outward unit surface normals (`f32`).
    pub normals: Vec<[f32; 3]>,
    /// Tile-local UV coordinates `[0, 1]^2`.
    pub uvs: Vec<[f32; 2]>,
    /// Vertex colors in linear RGBA (`[r, g, b, a]`).
    pub colors: Vec<[f32; 4]>,
    /// Watertight stitched triangle indices (`CCW` outward winding).
    pub indices: Vec<u32>,
}

/// Complete CPU-built tile geometry bundle ready for GPU upload.
#[derive(Debug, Clone, PartialEq)]
pub struct BuiltTileMeshes {
    /// Quadtree tile identifier.
    pub tile: TileId,
    /// Edge-stitching configuration used for `indices`.
    pub coarser_edges: CoarserEdges,
    /// Tile-local floating origin at the tile center in `f64` ECEF meters.
    pub tile_origin: LocalOrigin,
    /// Terrain revision this mesh was built against.
    pub revision: u64,
    /// Displaced terrain mesh with physical-geographic or bathymetric bed vertex colors.
    pub terrain_mesh: TileColoredMesh,
    /// Optional stitched water surface mesh when oceans, lakes, or rivers exist on this tile.
    pub water_mesh: Option<TileColoredMesh>,
}

/// Converts a [`TileColoredMesh`] into a Bevy [`Mesh`] with `ATTRIBUTE_COLOR`.
#[must_use]
pub fn to_bevy_mesh(tile_mesh: TileColoredMesh) -> Mesh {
    let mut mesh = Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::default(),
    );
    mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, tile_mesh.positions);
    mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, tile_mesh.normals);
    mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, tile_mesh.uvs);
    mesh.insert_attribute(Mesh::ATTRIBUTE_COLOR, tile_mesh.colors);
    mesh.insert_indices(Indices::U32(tile_mesh.indices));
    mesh
}

/// Builds displaced terrain and optional stitched water meshes in tile-local coordinates
/// anchored at `tile_origin` so camera `LocalOrigin` rebases require zero mesh rebuilds.
#[must_use]
pub fn build_tile_meshes(
    planet: PlanetSpec,
    tile: TileId,
    coarser_edges: CoarserEdges,
    revision: u64,
    sampler: &dyn ElevationSampler,
    world: Option<&TerrainWorld>,
) -> BuiltTileMeshes {
    let r_base = planet.radius_meters();
    let min_r = 1.0_f64.min(r_base * 0.5);
    let center_unit = face_uv_to_unit(
        tile.face,
        tile.tile_uv_to_face_uv(TileUv::clamped(0.5, 0.5)),
    );
    let center_elev = sample_clamped_elevation(planet, center_unit, sampler);
    let tile_center_ecef = center_unit.scale((r_base + center_elev).max(min_r));
    let tile_origin = LocalOrigin::new(tile_center_ecef).unwrap_or(LocalOrigin {
        ecef: EcefMeters::ZERO,
    });

    let f64_mesh = build_tile_mesh_f64_with_elevation(planet, tile, coarser_edges, sampler);
    let n = f64_mesh.grid_segments;
    let vertex_count = f64_mesh.positions_ecef.len();

    let terrain_positions: Vec<[f32; 3]> = f64_mesh
        .positions_ecef
        .iter()
        .map(|&p| tile_origin.to_render_vec3(p).to_array())
        .collect();

    let Some(world) = world else {
        let neutral = Color::srgb(0.62, 0.65, 0.68).to_linear().to_f32_array();
        let terrain_mesh = TileColoredMesh {
            positions: terrain_positions,
            normals: f64_mesh.normals,
            uvs: f64_mesh.uvs,
            colors: vec![neutral; vertex_count],
            indices: f64_mesh.indices,
        };
        return BuiltTileMeshes {
            tile,
            coarser_edges,
            tile_origin,
            revision,
            terrain_mesh,
            water_mesh: None,
        };
    };

    let (terrain_colors, has_water, shore_hint_rgb) =
        sample_terrain_colors(tile, n, &f64_mesh.normals, world);

    // Zero-allocation fast reject for dry tiles (including high-elevation dry land).
    let water_mesh = if has_water {
        Some(build_water_mesh(
            planet,
            tile,
            tile_origin,
            &f64_mesh.positions_ecef,
            f64_mesh.uvs.clone(),
            f64_mesh.indices.clone(),
            world,
            shore_hint_rgb,
        ))
    } else {
        None
    };

    BuiltTileMeshes {
        tile,
        coarser_edges,
        tile_origin,
        revision,
        terrain_mesh: TileColoredMesh {
            positions: terrain_positions,
            normals: f64_mesh.normals,
            uvs: f64_mesh.uvs,
            colors: terrain_colors,
            indices: f64_mesh.indices,
        },
        water_mesh,
    }
}

fn sample_terrain_colors(
    tile: TileId,
    n: u32,
    normals: &[[f32; 3]],
    world: &TerrainWorld,
) -> (Vec<[f32; 4]>, bool, [f32; 3]) {
    let inv_n = 1.0 / f64::from(n);
    let mut colors = Vec::with_capacity(normals.len());
    let mut has_water = false;
    let mut shore_hint_rgb = [0.08_f32, 0.36, 0.48];
    let mut idx = 0_usize;

    for iy in 0..=n {
        let v = f64::from(iy) * inv_n;
        for ix in 0..=n {
            let u = f64::from(ix) * inv_n;
            let unit_dir =
                face_uv_to_unit(tile.face, tile.tile_uv_to_face_uv(TileUv::clamped(u, v)));
            let normal = normals[idx];
            let dot = (f64::from(normal[0]) * unit_dir.x
                + f64::from(normal[1]) * unit_dir.y
                + f64::from(normal[2]) * unit_dir.z)
                .clamp(0.0, 1.0);
            let slope_sin = (1.0 - dot * dot).max(0.0).sqrt();

            let sample = world.sample_render_vertex(unit_dir, slope_sin);
            let srgb = match sample.water.kind {
                WaterBodyKind::Dry => sample.albedo_rgb,
                WaterBodyKind::Ocean => {
                    has_water = true;
                    shore_hint_rgb = [0.09, 0.42, 0.52];
                    seabed_srgb(sample.water.depth_m, slope_sin)
                }
                WaterBodyKind::River(_) => {
                    has_water = true;
                    shore_hint_rgb = [0.06, 0.29, 0.42];
                    riverbed_srgb(sample.water.depth_m)
                }
                WaterBodyKind::Lake(_) => {
                    has_water = true;
                    shore_hint_rgb = if sample.water.is_freshwater {
                        [0.04, 0.24, 0.43]
                    } else {
                        [0.06, 0.31, 0.38]
                    };
                    lakebed_srgb(sample.water.depth_m)
                }
            };
            colors.push(srgb_to_linear_rgba(srgb));
            idx += 1;
        }
    }

    (colors, has_water, shore_hint_rgb)
}

#[expect(
    clippy::too_many_arguments,
    clippy::cast_possible_truncation,
    reason = "water mesh builder bundles tile geometry, world sampler, and shore color hint"
)]
fn build_water_mesh(
    planet: PlanetSpec,
    tile: TileId,
    tile_origin: LocalOrigin,
    terrain_positions_ecef: &[EcefMeters],
    uvs: Vec<[f32; 2]>,
    indices: Vec<u32>,
    world: &TerrainWorld,
    shore_hint_rgb: [f32; 3],
) -> TileColoredMesh {
    let n = planet.grid_segments();
    let inv_n = 1.0 / f64::from(n);
    let r_base = planet.radius_meters();
    let min_r = 1.0_f64.min(r_base * 0.5);
    let cell_arc = planet.cell_arc_length_meters(tile.lod);
    let tuck_m = (cell_arc * 0.08).clamp(0.5, 45.0);
    let vertex_count = terrain_positions_ecef.len();

    let mut positions = Vec::with_capacity(vertex_count);
    let mut normals = Vec::with_capacity(vertex_count);
    let mut colors = Vec::with_capacity(vertex_count);
    let mut idx = 0_usize;

    for iy in 0..=n {
        let v = f64::from(iy) * inv_n;
        for ix in 0..=n {
            let u = f64::from(ix) * inv_n;
            let unit_dir =
                face_uv_to_unit(tile.face, tile.tile_uv_to_face_uv(TileUv::clamped(u, v)));
            let water = world.sample_water(unit_dir);
            let (ecef, rgb) = if water.kind == WaterBodyKind::Dry {
                let terrain_r = terrain_positions_ecef[idx].norm();
                let tucked_r = (terrain_r - tuck_m).max(min_r);
                (unit_dir.scale(tucked_r), shore_hint_rgb)
            } else {
                let surf_r = (r_base + water.surface_elevation_m).max(min_r);
                (unit_dir.scale(surf_r), water_surface_srgb(water))
            };
            positions.push(tile_origin.to_render_vec3(ecef).to_array());
            normals.push([unit_dir.x as f32, unit_dir.y as f32, unit_dir.z as f32]);
            colors.push(srgb_to_linear_rgba(rgb));
            idx += 1;
        }
    }

    TileColoredMesh {
        positions,
        normals,
        uvs,
        colors,
        indices,
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
