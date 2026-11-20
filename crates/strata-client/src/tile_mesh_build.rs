use std::sync::atomic::{AtomicBool, Ordering};

use bevy::asset::RenderAssetUsages;
use bevy::mesh::{Indices, PrimitiveTopology};
use bevy::prelude::*;
use strata_globe::{
    EcefMeters, ElevationSampler, LocalOrigin, PlanetSpec, TileId, TileUv, canonical_tangent_basis,
    face_uv_to_unit,
};
use strata_terrain::{TerrainWorld, TileVertexBase};

use crate::terrain::sample_clamped_elevation;
use crate::tile_mesh_colors::{build_water_stage, compute_terrain_colors};

/// Single colored tile mesh (`terrain` or `water`) in tile-local `f32` coordinates
/// relative to [`BuiltTileMeshes::tile_origin`].
///
/// Triangle indices are decoupled from vertices and supplied at GPU upload time
/// from [`crate::tile_cache::StitchedIndexTable`] so edge-stitching changes never
/// trigger vertex rebuilds.
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
}

/// Complete CPU-built tile geometry bundle ready for GPU upload.
#[derive(Debug, Clone, PartialEq)]
pub struct BuiltTileMeshes {
    /// Quadtree tile identifier.
    pub tile: TileId,
    /// Tile-local floating origin at the tile center in `f64` ECEF meters.
    pub tile_origin: LocalOrigin,
    /// Terrain revision this mesh was built against.
    pub revision: u64,
    /// Displaced terrain mesh with physical-geographic or bathymetric bed vertex colors.
    pub terrain_mesh: TileColoredMesh,
    /// Optional water surface mesh when oceans, lakes, or rivers exist on this tile.
    pub water_mesh: Option<TileColoredMesh>,
}

/// Converts a [`TileColoredMesh`] into a Bevy [`Mesh`] with `ATTRIBUTE_COLOR` and stitched indices.
#[must_use]
pub fn to_bevy_mesh(tile_mesh: TileColoredMesh, indices: &[u32]) -> Mesh {
    let mut mesh = Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::default(),
    );
    mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, tile_mesh.positions);
    mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, tile_mesh.normals);
    mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, tile_mesh.uvs);
    mesh.insert_attribute(Mesh::ATTRIBUTE_COLOR, tile_mesh.colors);
    mesh.insert_indices(Indices::U32(indices.to_vec()));
    mesh
}

/// Builds displaced terrain and optional water meshes in tile-local coordinates
/// using a single pass over the vertex grid.
#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "non-cancellable builder convenience wrapper exercised by unit tests"
    )
)]
#[must_use]
pub fn build_tile_meshes(
    planet: PlanetSpec,
    tile: TileId,
    revision: u64,
    sampler: &dyn ElevationSampler,
    world: Option<&TerrainWorld>,
) -> BuiltTileMeshes {
    let terrain_stage = build_terrain_stage(planet, tile, sampler, world);
    let water_mesh = if terrain_stage.has_water {
        Some(build_water_stage(
            planet,
            tile,
            terrain_stage.tile_origin,
            &terrain_stage.unit_dirs,
            &terrain_stage.base_samples,
            terrain_stage.uvs.clone(),
            terrain_stage.shore_hint_rgb,
        ))
    } else {
        None
    };

    BuiltTileMeshes {
        tile,
        tile_origin: terrain_stage.tile_origin,
        revision,
        terrain_mesh: TileColoredMesh {
            positions: terrain_stage.positions,
            normals: terrain_stage.normals,
            uvs: terrain_stage.uvs,
            colors: terrain_stage.colors,
        },
        water_mesh,
    }
}

/// Cancellable single-pass tile builder for background streaming workers.
///
/// Checks `cancelled` before building terrain vertices and again before building
/// the water mesh so stale tasks abort immediately without wasted work.
#[must_use]
pub fn build_tile_meshes_cancellable(
    planet: PlanetSpec,
    tile: TileId,
    revision: u64,
    sampler: &dyn ElevationSampler,
    world: Option<&TerrainWorld>,
    cancelled: &AtomicBool,
) -> Option<BuiltTileMeshes> {
    if cancelled.load(Ordering::Relaxed) {
        return None;
    }
    let terrain_stage = build_terrain_stage(planet, tile, sampler, world);
    let water_mesh = if terrain_stage.has_water {
        if cancelled.load(Ordering::Relaxed) {
            return None;
        }
        Some(build_water_stage(
            planet,
            tile,
            terrain_stage.tile_origin,
            &terrain_stage.unit_dirs,
            &terrain_stage.base_samples,
            terrain_stage.uvs.clone(),
            terrain_stage.shore_hint_rgb,
        ))
    } else {
        None
    };

    Some(BuiltTileMeshes {
        tile,
        tile_origin: terrain_stage.tile_origin,
        revision,
        terrain_mesh: TileColoredMesh {
            positions: terrain_stage.positions,
            normals: terrain_stage.normals,
            uvs: terrain_stage.uvs,
            colors: terrain_stage.colors,
        },
        water_mesh,
    })
}

struct TerrainStageOutput {
    tile_origin: LocalOrigin,
    unit_dirs: Vec<EcefMeters>,
    base_samples: Vec<TileVertexBase>,
    positions: Vec<[f32; 3]>,
    normals: Vec<[f32; 3]>,
    uvs: Vec<[f32; 2]>,
    colors: Vec<[f32; 4]>,
    has_water: bool,
    shore_hint_rgb: [f32; 3],
}

#[expect(
    clippy::cast_possible_truncation,
    reason = "UVs and outward surface normals intentionally narrow to f32 vertex attributes"
)]
fn build_terrain_stage(
    planet: PlanetSpec,
    tile: TileId,
    sampler: &dyn ElevationSampler,
    world: Option<&TerrainWorld>,
) -> TerrainStageOutput {
    let n = planet.grid_segments();
    let stride = (n + 1) as usize;
    let vertex_count = stride * stride;
    let inv_n = 1.0 / f64::from(n);
    let r_base = planet.radius_meters();
    let min_r = 1.0_f64.min(r_base * 0.5);

    let mut unit_dirs = Vec::with_capacity(vertex_count);
    let mut positions_ecef = Vec::with_capacity(vertex_count);
    let mut uvs = Vec::with_capacity(vertex_count);
    let mut base_samples = if world.is_some() {
        Vec::with_capacity(vertex_count)
    } else {
        Vec::new()
    };

    // Single-pass vertex sampling: each grid vertex queries TerrainWorld exactly once.
    for iy in 0..=n {
        let v = f64::from(iy) * inv_n;
        for ix in 0..=n {
            let u = f64::from(ix) * inv_n;
            let unit_dir =
                face_uv_to_unit(tile.face, tile.tile_uv_to_face_uv(TileUv::clamped(u, v)));
            let elev = if let Some(w) = world {
                let base = w.sample_tile_vertex_base(unit_dir);
                let clamped = if base.elevation_m.is_finite() {
                    base.elevation_m.max(min_r - r_base)
                } else {
                    0.0
                };
                base_samples.push(base);
                clamped
            } else {
                sample_clamped_elevation(planet, unit_dir, sampler)
            };
            positions_ecef.push(unit_dir.scale((r_base + elev).max(min_r)));
            unit_dirs.push(unit_dir);
            uvs.push([u as f32, v as f32]);
        }
    }

    let tile_center_ecef = if n.is_multiple_of(2) {
        let half = (n / 2) as usize;
        positions_ecef[half * stride + half]
    } else {
        let center_unit = face_uv_to_unit(
            tile.face,
            tile.tile_uv_to_face_uv(TileUv::clamped(0.5, 0.5)),
        );
        let center_elev = sample_clamped_elevation(planet, center_unit, sampler);
        center_unit.scale((r_base + center_elev).max(min_r))
    };
    let tile_origin = LocalOrigin::new(tile_center_ecef).unwrap_or(LocalOrigin {
        ecef: EcefMeters::ZERO,
    });

    let positions: Vec<[f32; 3]> = positions_ecef
        .iter()
        .map(|&p| tile_origin.to_render_vec3(p).to_array())
        .collect();

    let normals = compute_tile_normals(
        planet,
        tile,
        n,
        stride,
        r_base,
        min_r,
        &unit_dirs,
        &positions_ecef,
        sampler,
        world,
    );

    let (colors, has_water, shore_hint_rgb) = if world.is_some() {
        compute_terrain_colors(&unit_dirs, &normals, &base_samples)
    } else {
        let neutral = Color::srgb(0.62, 0.65, 0.68).to_linear().to_f32_array();
        (vec![neutral; vertex_count], false, [0.08_f32, 0.36, 0.48])
    };

    TerrainStageOutput {
        tile_origin,
        unit_dirs,
        base_samples,
        positions,
        normals,
        uvs,
        colors,
        has_water,
        shore_hint_rgb,
    }
}

#[expect(
    clippy::too_many_arguments,
    clippy::cast_possible_truncation,
    reason = "hybrid interior/perimeter normal kernel converts unit ECEF normals to f32"
)]
fn compute_tile_normals(
    planet: PlanetSpec,
    tile: TileId,
    n: u32,
    stride: usize,
    r_base: f64,
    min_r: f64,
    unit_dirs: &[EcefMeters],
    positions_ecef: &[EcefMeters],
    sampler: &dyn ElevationSampler,
    world: Option<&TerrainWorld>,
) -> Vec<[f32; 3]> {
    if world.is_none() && sampler.global_range().is_flat() {
        return unit_dirs
            .iter()
            .map(|u| [u.x as f32, u.y as f32, u.z as f32])
            .collect();
    }

    if let Some(surface) = world.and_then(TerrainWorld::surface_data)
        && surface.earth_water().is_some()
    {
        return unit_dirs
            .iter()
            .map(|&unit| {
                let normal = surface.earth_shading_normal(unit, r_base).unwrap_or(unit);
                [normal.x as f32, normal.y as f32, normal.z as f32]
            })
            .collect();
    }

    let d_theta = (planet.cell_arc_length_meters(tile.lod) / r_base).clamp(1e-7, 0.05);
    let (sin_d, cos_d) = d_theta.sin_cos();
    let mut normals = Vec::with_capacity(unit_dirs.len());

    for iy in 0..=n {
        let row = iy as usize;
        for ix in 0..=n {
            let col = ix as usize;
            let idx = row * stride + col;
            let unit_dir = unit_dirs[idx];

            let normal = if ix == 0 || ix == n || iy == 0 || iy == n {
                perimeter_canonical_normal(
                    planet, r_base, min_r, unit_dir, sin_d, cos_d, sampler, world,
                )
            } else {
                let dp_u = positions_ecef[idx + 1] - positions_ecef[idx - 1];
                let dp_v = positions_ecef[idx + stride] - positions_ecef[idx - stride];
                dp_u.cross(dp_v)
                    .normalized()
                    .ok()
                    .filter(|n_vec| n_vec.dot(unit_dir) > 0.0)
                    .unwrap_or(unit_dir)
            };
            normals.push([normal.x as f32, normal.y as f32, normal.z as f32]);
        }
    }
    normals
}

#[expect(
    clippy::too_many_arguments,
    reason = "seam-canonical tangent-plane normal evaluation shares precomputed angular step"
)]
fn perimeter_canonical_normal(
    planet: PlanetSpec,
    r_base: f64,
    min_r: f64,
    unit_dir: EcefMeters,
    sin_d: f64,
    cos_d: f64,
    sampler: &dyn ElevationSampler,
    world: Option<&TerrainWorld>,
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

    let sample_pos = |u: EcefMeters| -> EcefMeters {
        let elev = if let Some(w) = world {
            let e = w.sample_elevation_unit_prevalidated(u);
            if e.is_finite() {
                e.max(min_r - r_base)
            } else {
                0.0
            }
        } else {
            sample_clamped_elevation(planet, u, sampler)
        };
        u.scale((r_base + elev).max(min_r))
    };

    let dp1 = sample_pos(u1_pos) - sample_pos(u1_neg);
    let dp2 = sample_pos(u2_pos) - sample_pos(u2_neg);
    dp1.cross(dp2)
        .normalized()
        .ok()
        .filter(|n_vec| n_vec.dot(unit_dir) > 0.0)
        .unwrap_or(unit_dir)
}
