use std::collections::{HashMap, HashSet};

use strata_globe::{
    CoarserEdges, EcefMeters, GlobeError, LocalOrigin, RenderVec3, SelectedTile, TileEdge, TileId,
    build_stitched_indices, build_tile_mesh_f64_with_elevation, ecef_to_tile_uv,
};
use strata_terrain::{TerrainWorld, lat_lon_to_unit};

use crate::tile_mesh_build::build_tile_meshes;
use crate::tile_streaming::{MAX_CACHED_TILES, MAX_DISPATCH_PER_FRAME, TileStreamPlanner};

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
    let built = build_tile_meshes(planet, tile, CoarserEdges::NONE, 1, &world, Some(&world));

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
fn ocean_lake_and_river_tiles_produce_stitched_water_meshes() -> Result<(), GlobeError> {
    let world = TerrainWorld::earth_default();
    let planet = world.planet();
    let coarser = CoarserEdges::NONE
        .with_edge(TileEdge::Bottom, true)
        .with_edge(TileEdge::Right, true);
    let expected_indices = build_stitched_indices(planet.grid_segments(), coarser);

    let cases = [
        ("Pacific Ocean", tile_at(&world, 0.0, -150.0, 4)?),
        ("Lake Baikal", tile_at(&world, 53.5, 108.2, 7)?),
        ("Amazon River", tile_at(&world, -1.8, -54.0, 10)?),
    ];

    for (label, tile) in cases {
        let built = build_tile_meshes(planet, tile, coarser, 0, &world, Some(&world));
        let Some(water) = built.water_mesh else {
            return Err(GlobeError::InvalidTileCoord {
                lod: tile.lod,
                x: tile.x,
                y: tile.y,
            });
        };
        assert_eq!(
            water.indices, expected_indices,
            "{label} water mesh indices must match CoarserEdges stitching"
        );
        assert_eq!(water.colors.len(), water.positions.len());
        assert_eq!(water.normals.len(), water.positions.len());
        assert_valid_linear_colors(&water.colors);
        assert_valid_linear_colors(&built.terrain_mesh.colors);
    }
    Ok(())
}

#[test]
fn tile_local_origin_transform_reconstructs_ecef_within_1mm_after_rebase() -> Result<(), GlobeError>
{
    let world = TerrainWorld::earth_default();
    let planet = world.planet();
    let tile = tile_at(&world, 28.0, 86.9, 12)?;

    // Build tile mesh once in tile-local coordinates.
    let built = build_tile_meshes(planet, tile, CoarserEdges::NONE, 0, &world, Some(&world));
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
fn stream_planner_keeps_ancestor_fallback_and_enforces_budgets_and_lru() -> Result<(), GlobeError> {
    let parent = TileId::new(strata_globe::CubeFace::PosZ, 2, 1, 1)?;
    let Some(children) = parent.children() else {
        return Err(GlobeError::InvalidTileCoord {
            lod: parent.lod,
            x: parent.x,
            y: parent.y,
        });
    };

    let desired_children: Vec<SelectedTile> = children
        .iter()
        .enumerate()
        .map(|(i, &id)| SelectedTile {
            id,
            coarser_edges: CoarserEdges::NONE,
            distance_meters: 10_000.0 + f64::from(u32::try_from(i).unwrap_or(0)) * 500.0,
            sse_px: 4.0,
        })
        .collect();

    // 1. Zoom-in fallback: parent remains visible while children are pending or partially built.
    let mut ready = HashSet::from([parent]);
    let vis_0 = TileStreamPlanner::compute_visible_tiles(&desired_children, &ready);
    assert_eq!(vis_0, HashSet::from([parent]));

    ready.insert(children[0]);
    ready.insert(children[1]);
    let vis_partial = TileStreamPlanner::compute_visible_tiles(&desired_children, &ready);
    assert_eq!(vis_partial, HashSet::from([parent]));

    ready.insert(children[2]);
    ready.insert(children[3]);
    let vis_complete = TileStreamPlanner::compute_visible_tiles(&desired_children, &ready);
    assert_eq!(vis_complete, HashSet::from(children));

    // 2. Zoom-out fallback: ready children stay visible while coarse parent builds.
    let desired_parent = [SelectedTile {
        id: parent,
        coarser_edges: CoarserEdges::NONE,
        distance_meters: 50_000.0,
        sse_px: 2.0,
    }];
    let ready_children_only = HashSet::from(children);
    let vis_zoom_out =
        TileStreamPlanner::compute_visible_tiles(&desired_parent, &ready_children_only);
    assert_eq!(vis_zoom_out, HashSet::from(children));

    // 3. Per-frame dispatch budget prioritizes closest tiles and skips up-to-date cached/in-flight tiles.
    let planner = TileStreamPlanner::new(crate::tile_streaming::TileStreamBudget::default());
    let mut many_desired = Vec::new();
    for y in 0..5_u32 {
        for x in 0..4_u32 {
            let id = TileId::new(strata_globe::CubeFace::PosX, 5, x, y)?;
            many_desired.push(SelectedTile {
                id,
                coarser_edges: CoarserEdges::NONE,
                distance_meters: f64::from(20 - (y * 4 + x)) * 1_000.0,
                sse_px: 5.0,
            });
        }
    }
    let cached_edges = HashMap::from([(many_desired[19].id, CoarserEdges::NONE)]);
    let in_flight_edges = HashMap::from([(many_desired[18].id, CoarserEdges::NONE)]);
    let dispatches = planner.plan_dispatches(&many_desired, &cached_edges, &in_flight_edges);
    assert_eq!(dispatches.len(), MAX_DISPATCH_PER_FRAME);
    for pair in dispatches.windows(2) {
        assert!(pair[0].distance_meters <= pair[1].distance_meters);
    }
    assert!(!dispatches.iter().any(|d| d.id == many_desired[19].id));
    assert!(!dispatches.iter().any(|d| d.id == many_desired[18].id));

    // 4. LRU eviction keeps cache capped at MAX_CACHED_TILES and never evicts visible tiles.
    let total = MAX_CACHED_TILES + 8;
    let mut last_used = HashMap::with_capacity(total);
    let mut visible_lru = HashSet::new();
    let mut all_ids = Vec::with_capacity(total);
    for idx in 0..total {
        let lod = 10_u8;
        let x = u32::try_from(idx % 512).unwrap_or(0);
        let y = u32::try_from(idx / 512).unwrap_or(0);
        let id = TileId::new(strata_globe::CubeFace::PosY, lod, x, y)?;
        last_used.insert(id, u64::try_from(idx + 1).unwrap_or(1));
        if idx < 5 {
            visible_lru.insert(id);
        }
        all_ids.push(id);
    }
    let evicted = planner.plan_evictions(&last_used, &visible_lru);
    assert_eq!(evicted.len(), 8);
    for vis_id in &visible_lru {
        assert!(!evicted.contains(vis_id));
    }
    for (offset, expected_id) in all_ids.iter().skip(5).take(8).enumerate() {
        assert_eq!(evicted[offset], *expected_id);
    }

    // 5. Verify TerrainElevation::set_world switches from flat to Earth TerrainWorld and bumps revision.
    let mut terrain = crate::terrain::TerrainElevation::default();
    assert!(terrain.world().is_none());
    assert_eq!(terrain.revision(), 0);
    terrain.set_world(std::sync::Arc::new(TerrainWorld::earth_default()));
    assert!(terrain.world().is_some());
    assert_eq!(terrain.revision(), 1);
    Ok(())
}
