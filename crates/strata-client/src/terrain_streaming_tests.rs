use std::collections::{HashMap, HashSet};
use std::sync::atomic::AtomicBool;

use bevy::mesh::Indices;
use bevy::prelude::*;
use strata_globe::{
    CoarserEdges, CubeFace, GlobeError, SelectedTile, TileEdge, TileId, build_stitched_indices,
    ecef_to_tile_uv,
};
use strata_terrain::{TerrainWorld, lat_lon_to_unit};

use crate::tile_cache::{CachedTile, StitchedIndexTable, restitch_cached_tile};
use crate::tile_mesh_build::{build_tile_meshes, build_tile_meshes_cancellable, to_bevy_mesh};
use crate::tile_streaming::{
    MAX_CACHED_TILES, MAX_IN_FLIGHT_TASKS, TileStreamBudget, TileStreamPlanner,
};

#[test]
fn stream_planner_keeps_ancestor_fallback_and_enforces_budgets_and_lru() -> Result<(), GlobeError> {
    let parent = TileId::new(CubeFace::PosZ, 2, 1, 1)?;
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
    assert_eq!(
        TileStreamPlanner::compute_visible_tiles(&desired_children, &ready),
        HashSet::from([parent])
    );

    ready.insert(children[0]);
    ready.insert(children[1]);
    assert_eq!(
        TileStreamPlanner::compute_visible_tiles(&desired_children, &ready),
        HashSet::from([parent])
    );

    ready.insert(children[2]);
    ready.insert(children[3]);
    assert_eq!(
        TileStreamPlanner::compute_visible_tiles(&desired_children, &ready),
        HashSet::from(children)
    );

    // 2. Zoom-out fallback: ready children stay visible while coarse parent builds.
    let desired_parent = [SelectedTile {
        id: parent,
        coarser_edges: CoarserEdges::NONE,
        distance_meters: 50_000.0,
        sse_px: 2.0,
    }];
    let ready_children_only = HashSet::from(children);
    assert_eq!(
        TileStreamPlanner::compute_visible_tiles(&desired_parent, &ready_children_only),
        HashSet::from(children)
    );

    // 3. Dispatch budget prioritizes base-shell tiles (lod <= min_pinned_lod), then closest,
    //    skips cached/in-flight tiles regardless of coarser_edges, and respects max_in_flight.
    let planner = TileStreamPlanner::new(TileStreamBudget::default());
    let mut many_desired = Vec::new();
    for y in 0..5_u32 {
        for x in 0..4_u32 {
            let id = TileId::new(CubeFace::PosX, 5, x, y)?;
            many_desired.push(SelectedTile {
                id,
                coarser_edges: CoarserEdges::NONE,
                distance_meters: f64::from(20 - (y * 4 + x)) * 1_000.0,
                sse_px: 5.0,
            });
        }
    }
    let base_shell_far = TileId::new(CubeFace::NegZ, 1, 0, 0)?;
    many_desired.push(SelectedTile {
        id: base_shell_far,
        coarser_edges: CoarserEdges::NONE,
        distance_meters: 9_000_000.0,
        sse_px: 1.0,
    });
    let cached_tiles = HashSet::from([many_desired[19].id]);
    let in_flight_tiles = HashSet::from([many_desired[18].id]);
    let dispatches = planner.plan_dispatches(&many_desired, &cached_tiles, &in_flight_tiles, 1);
    assert_eq!(
        dispatches.len(),
        MAX_IN_FLIGHT_TASKS - in_flight_tiles.len()
    );
    assert_eq!(dispatches[0].id, base_shell_far);
    assert!(!dispatches.iter().any(|d| d.id == many_desired[19].id));
    assert!(!dispatches.iter().any(|d| d.id == many_desired[18].id));
    Ok(())
}

#[test]
fn stream_planner_evicts_lru_without_touching_pinned_or_visible() -> Result<(), GlobeError> {
    let planner = TileStreamPlanner::new(TileStreamBudget::default());

    // LRU eviction caps cache at MAX_CACHED_TILES and NEVER evicts visible or pinned tiles.
    let total = MAX_CACHED_TILES + 8;
    let mut last_used = HashMap::with_capacity(total);
    let mut visible_lru = HashSet::new();
    let mut all_ids = Vec::with_capacity(total);
    let pinned_tile = TileId::new(CubeFace::PosY, 1, 0, 0)?;
    last_used.insert(pinned_tile, 0);
    for idx in 1..total {
        let x = u32::try_from(idx % 512).unwrap_or(0);
        let y = u32::try_from(idx / 512).unwrap_or(0);
        let id = TileId::new(CubeFace::PosY, 10, x, y)?;
        last_used.insert(id, u64::try_from(idx + 1).unwrap_or(1));
        if idx <= 4 {
            visible_lru.insert(id);
        }
        all_ids.push(id);
    }
    let evicted = planner.plan_evictions(&last_used, &visible_lru, 1);
    assert_eq!(evicted.len(), 8);
    assert!(!evicted.contains(&pinned_tile));
    for vis_id in &visible_lru {
        assert!(!evicted.contains(vis_id));
    }
    for (offset, expected_id) in all_ids.iter().skip(4).take(8).enumerate() {
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

#[test]
fn zero_rebuild_edge_stitching_swaps_indices_without_dispatch_or_stale_cancel()
-> Result<(), GlobeError> {
    let world = TerrainWorld::earth_default();
    let planet = world.planet();
    let unit = lat_lon_to_unit(53.5, 108.2);
    let (tile, _, _) = ecef_to_tile_uv(planet, unit.scale(planet.radius_meters()), 7)?;
    let built = build_tile_meshes(planet, tile, 0, &world, Some(&world));
    let table = StitchedIndexTable::new(planet.grid_segments());

    let edges_a = CoarserEdges::NONE;
    let edges_b = CoarserEdges::NONE
        .with_edge(TileEdge::Top, true)
        .with_edge(TileEdge::Left, true);

    let mut meshes = Assets::<Mesh>::default();
    let terrain_handle = meshes.add(to_bevy_mesh(built.terrain_mesh, table.indices(edges_a)));
    let water_handle = built
        .water_mesh
        .map(|w| meshes.add(to_bevy_mesh(w, table.indices(edges_a))));

    let mut cached = CachedTile {
        terrain_entity: Entity::PLACEHOLDER,
        terrain_mesh: terrain_handle.clone(),
        water_entity: water_handle.as_ref().map(|_| Entity::PLACEHOLDER),
        water_mesh: water_handle.clone(),
        coarser_edges: edges_a,
        tile_origin: built.tile_origin,
        last_used_frame: 1,
    };

    assert!(restitch_cached_tile(
        &mut meshes,
        &mut cached,
        edges_b,
        table.indices(edges_b)
    ));
    assert_eq!(cached.coarser_edges, edges_b);
    let expected_b = build_stitched_indices(planet.grid_segments(), edges_b);
    assert_eq!(
        meshes.get(&terrain_handle).and_then(Mesh::indices),
        Some(&Indices::U32(expected_b.clone()))
    );
    if let Some(w_handle) = water_handle {
        assert_eq!(
            meshes.get(&w_handle).and_then(Mesh::indices),
            Some(&Indices::U32(expected_b))
        );
    }

    // Changing coarser_edges neither triggers a new dispatch nor marks in-flight task stale.
    let planner = TileStreamPlanner::default();
    let desired = [SelectedTile {
        id: tile,
        coarser_edges: edges_b,
        distance_meters: 50_000.0,
        sse_px: 3.0,
    }];
    let cached_set = HashSet::from([tile]);
    let in_flight_set = HashSet::new();
    assert!(
        planner
            .plan_dispatches(&desired, &cached_set, &in_flight_set, 1)
            .is_empty()
    );
    let desired_map = HashMap::from([(tile, edges_b)]);
    assert!(!TileStreamPlanner::is_stale_task(
        tile,
        planet,
        0,
        &desired_map,
        planet,
        0
    ));
    Ok(())
}

#[test]
fn cancellable_tile_builder_aborts_immediately_when_flagged() -> Result<(), GlobeError> {
    let world = TerrainWorld::earth_default();
    let planet = world.planet();
    let tile = TileId::new(CubeFace::PosX, 4, 2, 2)?;

    let flag = AtomicBool::new(true);
    let aborted = build_tile_meshes_cancellable(planet, tile, 0, &world, Some(&world), &flag);
    assert!(aborted.is_none());

    let active_flag = AtomicBool::new(false);
    let built = build_tile_meshes_cancellable(planet, tile, 0, &world, Some(&world), &active_flag);
    assert!(built.is_some());
    Ok(())
}
