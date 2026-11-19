use std::collections::HashMap;
use std::error::Error;

use strata_globe::{
    AuthoritativePoint, CoarserEdges, ControlPointKind, CubeFace, EcefMeters, FaceUv, GlobeError,
    GlobeSnapshot, LocalOrigin, PlanetSpec, PointId, SnapshotChannel, SurfaceCoord, TileEdge,
    TileId, TileUv, WorldEpoch, build_stitched_indices, build_tile_mesh_f64, ecef_to_tile_uv,
    face_uv_to_unit,
};

#[test]
fn planet_spec_validation_and_non_earth_sizes() -> Result<(), GlobeError> {
    assert!(PlanetSpec::with_radius(0.0).is_err());
    assert!(PlanetSpec::with_radius(-100.0).is_err());
    assert!(PlanetSpec::with_radius(f64::NAN).is_err());
    assert!(PlanetSpec::with_radius(f64::INFINITY).is_err());
    assert!(PlanetSpec::new(1_000_000.0, 23, 16).is_err());
    assert!(PlanetSpec::new(1_000_000.0, 12, 15).is_err());
    assert!(PlanetSpec::new(1_000_000.0, 12, 2).is_err());

    let moon = PlanetSpec::new(1_737_400.0, 16, 16)?;
    let super_earth = PlanetSpec::new(12_742_000.0, 18, 16)?;

    for planet in [PlanetSpec::earth(), moon, super_earth] {
        let p = SurfaceCoord::from_degrees(23.5, -45.25, 0.0)?.to_ecef(planet)?;
        let (tile, uv, elev) = ecef_to_tile_uv(planet, p, planet.max_lod())?;
        let back = tile.tile_uv_to_ecef(planet, uv, elev);
        let err_m = p.distance_to(back);
        assert!(
            err_m <= 1e-6,
            "round-trip error {err_m} m exceeded 1 um for radius {}",
            planet.radius_meters()
        );
    }
    Ok(())
}

#[test]
fn round_trip_ecef_tile_uv_across_all_lods_and_control_points() -> Result<(), Box<dyn Error>> {
    let planet = PlanetSpec::earth();
    let r = planet.radius_meters();

    let mut test_points: Vec<(&str, EcefMeters)> = Vec::new();
    // Tile interior.
    test_points.push((
        "interior",
        face_uv_to_unit(CubeFace::PosZ, FaceUv::clamped(0.37, 0.61)).scale(r),
    ));
    // Intra-face seam and +/- 1 mm, +/- 1 m offsets.
    for delta_m in [0.0_f64, 1e-3, -1e-3, 1.0, -1.0] {
        let seam_angle = delta_m / r;
        let p = face_uv_to_unit(CubeFace::PosZ, FaceUv::clamped(0.5 + seam_angle, 0.35)).scale(r);
        test_points.push(("intra_face_seam_neighborhood", p));
    }
    // 2-face cube edge (+X / +Y) and +/- 1 mm offsets.
    for delta_m in [0.0_f64, 1e-3, -1e-3] {
        let edge_pt = EcefMeters::from_raw(1.0, 1.0 + delta_m / r, 0.25).normalized()?;
        test_points.push(("cube_edge_xy", edge_pt.scale(r)));
    }
    // 3-face cube corner (+X / +Y / +Z) and antipodal corner.
    test_points.push((
        "cube_corner_xyz",
        EcefMeters::from_raw(1.0, 1.0, 1.0).normalized()?.scale(r),
    ));
    test_points.push((
        "cube_corner_neg_xyz",
        EcefMeters::from_raw(-1.0, -1.0, -1.0)
            .normalized()?
            .scale(r),
    ));
    // Poles and antipode.
    test_points.push(("north_pole", EcefMeters::from_raw(0.0, r, 0.0)));
    test_points.push(("south_pole", EcefMeters::from_raw(0.0, -r, 0.0)));
    test_points.push(("antipode", EcefMeters::from_raw(0.0, 0.0, -r)));
    // Subsurface negative elevation.
    test_points.push((
        "subsurface",
        SurfaceCoord::from_degrees(-12.34, 141.22, -2_500.0)?.to_ecef(planet)?,
    ));

    let mut max_err_m = 0.0_f64;
    for lod in 0..=planet.max_lod() {
        for &(label, pt) in &test_points {
            let (tile, uv, elev) = ecef_to_tile_uv(planet, pt, lod)?;
            let round_tripped = tile.tile_uv_to_ecef(planet, uv, elev);
            let err_m = pt.distance_to(round_tripped);
            max_err_m = max_err_m.max(err_m);
            assert!(
                err_m <= 1e-6,
                "round-trip error {err_m:.3e} m > 1e-6 m at LOD {lod} ({label})"
            );
        }
    }
    assert!(max_err_m <= 1e-6);
    println!("METRIC round_trip_max_err_m={max_err_m:.3e}");
    Ok(())
}

#[test]
fn authoritative_f64_invariance_across_planet_local_cycles() -> Result<(), Box<dyn Error>> {
    let snapshot = GlobeSnapshot::reference_earth();
    let initial_positions: Vec<EcefMeters> = snapshot.points.iter().map(|p| p.position).collect();

    for cycle in 0..50_u32 {
        let alt = if cycle % 2 == 0 { 20_000_000.0 } else { 20.0 };
        for (idx, pt) in snapshot.points.iter().enumerate() {
            let normal = pt.position.normalized()?;
            let cam_pos = pt.position + normal.scale(alt);
            let origin = LocalOrigin::new(cam_pos)?;
            let _render = origin.to_render_vec3(pt.position);
            let drift = pt.position.distance_to(initial_positions[idx]);
            assert!(drift == 0.0, "authoritative f64 drift {drift} != 0.0");
        }
    }
    Ok(())
}

#[test]
fn cube_face_neighbor_involution_and_boundary_continuity() -> Result<(), Box<dyn Error>> {
    let planet = PlanetSpec::earth();
    for lod in [0_u8, 1, 2, 3, 4, 10, 12, 18, 22] {
        let dim = 1_u32 << lod;
        let indices: Vec<u32> = if lod <= 4 {
            (0..dim).collect()
        } else {
            vec![0, dim - 1]
        };
        for face in CubeFace::ALL {
            for edge in TileEdge::ALL {
                for &i in &indices {
                    let (x, y) = match edge {
                        TileEdge::Bottom => (i, 0),
                        TileEdge::Right => (dim - 1, i),
                        TileEdge::Top => (i, dim - 1),
                        TileEdge::Left => (0, i),
                    };
                    let tile = TileId::new(face, lod, x, y)?;
                    let nb = tile.neighbor(edge);
                    assert!(nb.crossed_face);
                    let back = nb.tile.neighbor(nb.shared_edge_on_neighbor);
                    assert_eq!(back.tile, tile);
                    assert_eq!(back.shared_edge_on_neighbor, edge);
                }
            }
        }
    }

    // Compare shared boundary points computed independently across 2-face edge (+X/+Y)
    // and 3-face corner (+X/+Y/+Z) in both f64 and local-scale f32.
    let east_face_tile = TileId::new(CubeFace::PosX, 12, 0, 4095)?;
    let north_face_tile = TileId::new(CubeFace::PosY, 12, 4095, 0)?;
    let prime_face_tile = TileId::new(CubeFace::PosZ, 12, 4095, 4095)?;

    let corner_from_east = east_face_tile.tile_uv_to_ecef(planet, TileUv::clamped(0.0, 1.0), 0.0);
    let corner_from_north = north_face_tile.tile_uv_to_ecef(planet, TileUv::clamped(1.0, 0.0), 0.0);
    let corner_from_prime = prime_face_tile.tile_uv_to_ecef(planet, TileUv::clamped(1.0, 1.0), 0.0);

    assert!(corner_from_east.distance_to(corner_from_north) <= 1e-6);
    assert!(corner_from_east.distance_to(corner_from_prime) <= 1e-6);

    let local_cam = corner_from_east + corner_from_east.normalized()?.scale(20.0);
    let origin = LocalOrigin::new(local_cam)?;
    let rel_from_east = origin.to_render_vec3(corner_from_east);
    let rel_from_north = origin.to_render_vec3(corner_from_north);
    let rel_from_prime = origin.to_render_vec3(corner_from_prime);
    let seam_delta_xy = ((rel_from_east.x - rel_from_north.x).powi(2)
        + (rel_from_east.y - rel_from_north.y).powi(2)
        + (rel_from_east.z - rel_from_north.z).powi(2))
    .sqrt();
    let corner_delta_xz = ((rel_from_east.x - rel_from_prime.x).powi(2)
        + (rel_from_east.y - rel_from_prime.y).powi(2)
        + (rel_from_east.z - rel_from_prime.z).powi(2))
    .sqrt();
    let delta_f64_east_north = corner_from_east.distance_to(corner_from_north);
    let delta_f64_east_prime = corner_from_east.distance_to(corner_from_prime);
    assert!(seam_delta_xy <= 1e-3);
    assert!(corner_delta_xz <= 1e-3);
    println!(
        "METRIC corner_f64_delta_m={:.3e} corner_f32_delta_m={:.3e}",
        delta_f64_east_north.max(delta_f64_east_prime),
        seam_delta_xy.max(corner_delta_xz)
    );
    Ok(())
}

#[test]
fn stitched_indices_watertight_across_all_16_edge_masks() {
    let n = 16_u32;
    let stride = n + 1;

    for mask in 0..16_u8 {
        let coarser = CoarserEdges::from_mask(mask);
        let indices = build_stitched_indices(n, coarser);
        assert!(indices.len().is_multiple_of(3));

        let mut total_area_2x = 0_i64;
        let mut directed_edges: HashMap<(u32, u32), usize> = HashMap::new();

        for tri in indices.chunks_exact(3) {
            let (i0, i1, i2) = (tri[0], tri[1], tri[2]);
            let (x0, y0) = (i64::from(i0 % stride), i64::from(i0 / stride));
            let (x1, y1) = (i64::from(i1 % stride), i64::from(i1 / stride));
            let (x2, y2) = (i64::from(i2 % stride), i64::from(i2 / stride));

            // Verify odd vertices on coarser edges are never referenced.
            for (vx, vy) in [(x0, y0), (x1, y1), (x2, y2)] {
                if coarser.bottom() && vy == 0 {
                    assert_eq!(
                        vx % 2,
                        0,
                        "odd vertex ({vx}, 0) used on coarser bottom edge"
                    );
                }
                if coarser.top() && vy == i64::from(n) {
                    assert_eq!(vx % 2, 0, "odd vertex ({vx}, {n}) used on coarser top edge");
                }
                if coarser.left() && vx == 0 {
                    assert_eq!(vy % 2, 0, "odd vertex (0, {vy}) used on coarser left edge");
                }
                if coarser.right() && vx == i64::from(n) {
                    assert_eq!(
                        vy % 2,
                        0,
                        "odd vertex ({n}, {vy}) used on coarser right edge"
                    );
                }
            }

            let area2 = (x1 - x0) * (y2 - y0) - (y1 - y0) * (x2 - x0);
            assert!(area2 > 0, "degenerate or CW triangle in mask {mask}");
            total_area_2x += area2;

            for (a, b) in [(i0, i1), (i1, i2), (i2, i0)] {
                *directed_edges.entry((a, b)).or_insert(0) += 1;
            }
        }

        // Exact normalized grid area must be n * n (2x area = 2 * n * n).
        assert_eq!(
            total_area_2x,
            i64::from(2 * n * n),
            "area mismatch for mask {mask}"
        );

        // Every directed edge appears once, and every unmatched half-edge is on the outer boundary.
        let mut boundary_edges = 0_u32;
        for (&(a, b), &count) in &directed_edges {
            assert_eq!(
                count, 1,
                "duplicate directed edge ({a}, {b}) in mask {mask}"
            );
            if !directed_edges.contains_key(&(b, a)) {
                boundary_edges += 1;
            }
        }

        let expected_boundary_edges = 4 * n
            - (u32::from(coarser.bottom())
                + u32::from(coarser.right())
                + u32::from(coarser.top())
                + u32::from(coarser.left()))
                * (n / 2);
        assert_eq!(boundary_edges, expected_boundary_edges);
    }
}

#[test]
fn stitched_lod_and_cube_face_mesh_edges_match_without_cracks() -> Result<(), GlobeError> {
    let planet = PlanetSpec::earth();

    // 1. LOD L vs LOD L+1 boundary across an intra-face seam.
    let coarse = TileId::new(CubeFace::PosZ, 8, 100, 100)?;
    let fine = TileId::new(CubeFace::PosZ, 9, 202, 200)?; // Left edge touches coarse's Right edge.
    let coarse_mesh = build_tile_mesh_f64(planet, coarse, CoarserEdges::NONE);
    let fine_mesh = build_tile_mesh_f64(
        planet,
        fine,
        CoarserEdges::NONE.with_edge(TileEdge::Left, true),
    );

    let n = planet.grid_segments();
    let stride = n + 1;
    // Fine left edge uses even vertices iy = 0, 2, ..., n, matching bottom half of coarse right edge (iy = 0..=n/2).
    for k in 0..=(n / 2) {
        let fine_pos = fine_mesh.positions_ecef[((2 * k) * stride) as usize];
        let coarse_pos = coarse_mesh.positions_ecef[(k * stride + n) as usize];
        assert!(
            fine_pos.distance_to(coarse_pos) <= 1e-6,
            "L/L+1 seam mismatch at step {k}"
        );
    }

    // 2. Cross-face seam (+Z Right edge vs +X Left edge) at same LOD.
    let dim = 1_u32 << 10;
    let prime_edge_tile = TileId::new(CubeFace::PosZ, 10, dim - 1, 512)?;
    let east_edge_tile = TileId::new(CubeFace::PosX, 10, 0, 512)?;
    let mesh_on_prime = build_tile_mesh_f64(planet, prime_edge_tile, CoarserEdges::NONE);
    let mesh_on_east = build_tile_mesh_f64(planet, east_edge_tile, CoarserEdges::NONE);
    for iy in 0..=n {
        let p_left = mesh_on_east.positions_ecef[(iy * stride) as usize];
        let p_right = mesh_on_prime.positions_ecef[(iy * stride + n) as usize];
        assert!(
            p_left.distance_to(p_right) <= 1e-6,
            "cross-face mesh vertex mismatch at row {iy}"
        );
    }
    Ok(())
}

#[test]
fn max_lod_surface_sagitta_within_5mm_budget() -> Result<(), GlobeError> {
    for planet in [
        PlanetSpec::earth(),
        PlanetSpec::new(1_737_400.0, 16, 16)?,
        PlanetSpec::new(12_742_000.0, 18, 16)?,
    ] {
        let tile = TileId::new(CubeFace::PosX, planet.max_lod(), 0, 0)?;
        let mesh = build_tile_mesh_f64(planet, tile, CoarserEdges::NONE);
        let mut max_sag_m = 0.0_f64;

        for tri in mesh.indices.chunks_exact(3) {
            let p0 = mesh.positions_ecef[tri[0] as usize];
            let p1 = mesh.positions_ecef[tri[1] as usize];
            let p2 = mesh.positions_ecef[tri[2] as usize];
            // Sample edge midpoints and centroid where radial chord sagitta is largest.
            let samples = [
                (p0 + p1).scale(0.5),
                (p1 + p2).scale(0.5),
                (p2 + p0).scale(0.5),
                (p0 + p1 + p2).scale(1.0 / 3.0),
            ];
            for s in samples {
                let sag = (planet.radius_meters() - s.norm()).abs();
                max_sag_m = max_sag_m.max(sag);
            }
        }

        assert!(
            max_sag_m <= 5e-3,
            "surface sagitta {max_sag_m:.6e} m > 5e-3 m for radius {}",
            planet.radius_meters()
        );
        let analytical = planet.tile_surface_sagitta_meters(planet.max_lod());
        assert!(analytical <= 5e-3);
        println!(
            "METRIC sagitta_r{:.0}_lod{}_sampled_m={max_sag_m:.3e} analytical_m={analytical:.3e}",
            planet.radius_meters(),
            planet.max_lod()
        );
    }
    Ok(())
}

#[test]
fn snapshot_channel_preserves_reader_isolation() {
    let planet = PlanetSpec::earth();
    let p_a = EcefMeters::from_raw(0.0, 0.0, planet.radius_meters());
    let p_b = EcefMeters::from_raw(100.0, 0.0, planet.radius_meters());

    let mk_snap = |tick: u64, pos: EcefMeters| GlobeSnapshot {
        epoch: WorldEpoch(1),
        tick,
        planet,
        points: vec![AuthoritativePoint {
            id: PointId(42),
            label: "Tracked".to_owned(),
            kind: ControlPointKind::WorldEntity,
            position: pos,
            elevation_m: 0.0,
        }],
    };

    let channel = SnapshotChannel::new(mk_snap(10, p_a));
    let held_tick_10 = channel.latest();
    channel.publish(mk_snap(11, p_a));
    channel.publish(mk_snap(12, p_b));

    let latest_tick_12 = channel.latest();
    assert_eq!(held_tick_10.tick, 10);
    assert_eq!(
        held_tick_10.point(PointId(42)).map(|p| p.position),
        Some(p_a)
    );
    assert_eq!(latest_tick_12.tick, 12);
    assert_eq!(
        latest_tick_12.point(PointId(42)).map(|p| p.position),
        Some(p_b)
    );
}
