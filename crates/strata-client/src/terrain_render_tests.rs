use bevy::mesh::Indices;
use strata_globe::{
    CoarserEdges, CubeFace, EcefMeters, GlobeError, LocalOrigin, RenderVec3, TileEdge, TileId,
    build_stitched_indices, build_tile_mesh_f64_with_elevation, ecef_to_tile_uv,
};
use strata_terrain::{TerrainWorld, lat_lon_to_unit};

use crate::tile_cache::StitchedIndexTable;
use crate::tile_mesh_build::{build_tile_meshes, to_bevy_mesh};

fn tile_at(
    world: &TerrainWorld,
    lat_deg: f64,
    lon_deg: f64,
    lod: u8,
) -> Result<TileId, GlobeError> {
    let planet = world.planet();
    let unit = lat_lon_to_unit(lat_deg, lon_deg);
    let (tile, _, _) = ecef_to_tile_uv(planet, unit.scale(planet.radius_meters()), lod)?;
    Ok(tile)
}

fn assert_valid_linear_colors(colors: &[[f32; 4]]) {
    assert!(!colors.is_empty());
    for c in colors {
        for &ch in c {
            assert!(ch.is_finite(), "vertex color channel must be finite");
            assert!((0.0..=1.0).contains(&ch), "linear RGBA out of [0, 1]: {ch}");
        }
        assert!((c[3] - 1.0).abs() <= 1e-6);
    }
}

#[test]
fn high_dry_himalayan_tile_produces_colored_terrain_without_water_mesh() -> Result<(), GlobeError> {
    let world = TerrainWorld::earth_default();
    let planet = world.planet();
    let tile = tile_at(&world, 30.0, 87.0, 8)?;
    let built = build_tile_meshes(planet, tile, 1, &world, Some(&world));

    assert!(strata_globe::ElevationSampler::tile_range(&world, planet, tile).min_meters > 0.0);
    assert_eq!(built.tile, tile);
    assert_eq!(built.revision, 1);
    assert_eq!(
        built.terrain_mesh.colors.len(),
        built.terrain_mesh.positions.len()
    );
    assert_valid_linear_colors(&built.terrain_mesh.colors);
    assert!(
        built.water_mesh.is_none(),
        "dry high-altitude Himalayan tile must fast-reject water_mesh"
    );
    Ok(())
}

#[test]
fn ocean_lake_and_river_tiles_produce_stitched_water_meshes()
-> Result<(), Box<dyn std::error::Error>> {
    let world = TerrainWorld::earth_default();
    let planet = world.planet();
    let coarser = CoarserEdges::NONE
        .with_edge(TileEdge::Bottom, true)
        .with_edge(TileEdge::Right, true);
    let expected_indices = build_stitched_indices(planet.grid_segments(), coarser);
    let table = StitchedIndexTable::new(planet.grid_segments());
    assert_eq!(table.indices(coarser), expected_indices.as_slice());

    let amazon = world
        .rivers()
        .iter()
        .filter(|river| river.name == "Amazonas")
        .flat_map(|river| &river.waypoints)
        .find(|point| {
            let water = world.sample_water(point.unit_dir);
            matches!(water.kind, strata_terrain::WaterBodyKind::River(_)) && water.depth_m > 3.0
        })
        .ok_or("the baked Amazonas profile has no deep river sample")?;
    let (river_tile, _, _) = ecef_to_tile_uv(planet, amazon.unit_dir, 13)?;
    let cases = [
        ("Pacific Ocean", tile_at(&world, 0.0, -150.0, 4)?),
        ("Lake Baikal", tile_at(&world, 53.5, 108.2, 7)?),
        ("Amazon River", river_tile),
    ];

    for (label, tile) in cases {
        let built = build_tile_meshes(planet, tile, 0, &world, Some(&world));
        let Some(water) = built.water_mesh else {
            return Err(format!("{label} at {tile:?} has no water mesh").into());
        };
        assert_eq!(water.colors.len(), water.positions.len());
        assert_eq!(water.normals.len(), water.positions.len());
        assert_valid_linear_colors(&water.colors);
        assert_valid_linear_colors(&built.terrain_mesh.colors);

        let bevy_water = to_bevy_mesh(water, table.indices(coarser));
        assert_eq!(
            bevy_water.indices(),
            Some(&Indices::U32(expected_indices.clone())),
            "{label} water mesh indices must match CoarserEdges stitching"
        );
    }
    Ok(())
}

#[test]
fn tile_local_origin_transform_reconstructs_ecef_within_1mm_after_rebase() -> Result<(), GlobeError>
{
    let world = TerrainWorld::earth_default();
    let planet = world.planet();
    let tile = tile_at(&world, 28.0, 86.9, 12)?;

    let built = build_tile_meshes(planet, tile, 0, &world, Some(&world));
    let truth = build_tile_mesh_f64_with_elevation(planet, tile, CoarserEdges::NONE, &world);

    let center_ecef = built.tile_origin.ecef;
    let rebase_origins = [
        LocalOrigin::new(center_ecef + EcefMeters::from_raw(2_500.0, -1_200.0, 900.0))?,
        LocalOrigin::new(center_ecef + EcefMeters::from_raw(-5_500.0, 3_400.0, -2_100.0))?,
    ];

    for origin in rebase_origins {
        let tile_translation = origin.to_render_vec3(built.tile_origin.ecef);
        let mut checked = 0_usize;
        for (idx, local_pos) in built.terrain_mesh.positions.iter().enumerate() {
            let truth_ecef = truth.positions_ecef[idx];
            if truth_ecef.distance_to(origin.ecef) > 10_000.0 {
                continue;
            }
            let combined = RenderVec3::new(
                tile_translation.x + local_pos[0],
                tile_translation.y + local_pos[1],
                tile_translation.z + local_pos[2],
            );
            let reconstructed = origin.reconstruct_ecef_f64(combined);
            let err_m = reconstructed.distance_to(truth_ecef);
            assert!(
                err_m <= 1e-3,
                "vertex {idx} ECEF error {err_m:.4e} m exceeded 1 mm after LocalOrigin rebase"
            );
            checked += 1;
        }
        assert!(
            checked > 0,
            "expected vertices within 10 km of camera origin"
        );
    }
    Ok(())
}

#[test]
fn single_pass_mesh_builder_matches_seam_positions_and_normals_across_tiles()
-> Result<(), GlobeError> {
    let world = TerrainWorld::earth_default();
    let planet = world.planet();
    let stride = (planet.grid_segments() + 1) as usize;
    let n = planet.grid_segments() as usize;

    // 1. Intra-face vertical seam: PosZ (x=1, y=1) right edge == PosZ (x=2, y=1) left edge.
    let left_tile = TileId::new(CubeFace::PosZ, 3, 1, 1)?;
    let right_tile = TileId::new(CubeFace::PosZ, 3, 2, 1)?;
    let left_mesh = build_tile_meshes(planet, left_tile, 0, &world, Some(&world));
    let right_mesh = build_tile_meshes(planet, right_tile, 0, &world, Some(&world));

    for iy in 0..=n {
        let l_idx = iy * stride + n;
        let r_idx = iy * stride;
        assert_eq!(
            left_mesh.terrain_mesh.normals[l_idx].map(f32::to_bits),
            right_mesh.terrain_mesh.normals[r_idx].map(f32::to_bits),
            "intra-face seam normal mismatch at iy={iy}"
        );
        let l_p = left_mesh.terrain_mesh.positions[l_idx];
        let r_p = right_mesh.terrain_mesh.positions[r_idx];
        let l_ecef = left_mesh.tile_origin.ecef
            + EcefMeters::from_raw(f64::from(l_p[0]), f64::from(l_p[1]), f64::from(l_p[2]));
        let r_ecef = right_mesh.tile_origin.ecef
            + EcefMeters::from_raw(f64::from(r_p[0]), f64::from(r_p[1]), f64::from(r_p[2]));
        assert!(
            l_ecef.distance_to(r_ecef) <= 0.25,
            "intra-face seam position gap at iy={iy}"
        );
    }

    // 2. Cross-face cube edge seam: PosZ right edge (u=1) == PosX left edge (u=0).
    let face_z = TileId::new(CubeFace::PosZ, 2, 3, 1)?;
    let face_x = TileId::new(CubeFace::PosX, 2, 0, 1)?;
    let mesh_z = build_tile_meshes(planet, face_z, 0, &world, Some(&world));
    let mesh_x = build_tile_meshes(planet, face_x, 0, &world, Some(&world));
    for iy in 0..=n {
        let z_idx = iy * stride + n;
        let x_idx = iy * stride;
        assert_eq!(
            mesh_z.terrain_mesh.normals[z_idx].map(f32::to_bits),
            mesh_x.terrain_mesh.normals[x_idx].map(f32::to_bits),
            "cube-edge seam normal mismatch at iy={iy}"
        );
    }
    Ok(())
}
