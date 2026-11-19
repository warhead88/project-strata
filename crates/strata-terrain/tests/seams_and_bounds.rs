use strata_globe::{
    CoarserEdges, CubeFace, EcefMeters, ElevationSampler, TileEdge, TileId, TileUv,
    build_tile_mesh_f64_with_elevation, face_uv_to_unit,
};
use strata_terrain::{TerrainWorld, lat_lon_to_unit};

#[test]
fn seam_and_corner_elevations_are_bit_identical() {
    let world = TerrainWorld::earth_default();
    let planet = world.planet();

    // Check 2-face edges across all 6 faces at LOD 3.
    for face in CubeFace::ALL {
        let edge_tiles = [
            TileId {
                face,
                lod: 3,
                x: 0,
                y: 0,
            },
            TileId {
                face,
                lod: 3,
                x: 7,
                y: 0,
            },
            TileId {
                face,
                lod: 3,
                x: 7,
                y: 7,
            },
            TileId {
                face,
                lod: 3,
                x: 0,
                y: 7,
            },
            TileId {
                face,
                lod: 3,
                x: 3,
                y: 7,
            },
        ];
        for tile in edge_tiles {
            let mesh = build_tile_mesh_f64_with_elevation(planet, tile, CoarserEdges::NONE, &world);
            for edge in TileEdge::ALL {
                let nb = tile.neighbor(edge);
                let nb_mesh =
                    build_tile_mesh_f64_with_elevation(planet, nb.tile, CoarserEdges::NONE, &world);
                let n = planet.grid_segments() as usize;
                let stride = n + 1;

                // Compare endpoints and midpoint on the shared edge via unit_dir.
                let samples = [0.0_f64, 0.25, 0.5, 0.75, 1.0];
                for &t in &samples {
                    let uv = match edge {
                        TileEdge::Bottom => TileUv::clamped(t, 0.0),
                        TileEdge::Right => TileUv::clamped(1.0, t),
                        TileEdge::Top => TileUv::clamped(t, 1.0),
                        TileEdge::Left => TileUv::clamped(0.0, t),
                    };
                    let unit = face_uv_to_unit(tile.face, tile.tile_uv_to_face_uv(uv));
                    let e1 = world.sample_elevation(unit);
                    let e2 = world.sample_elevation(unit);
                    assert_eq!(e1.to_bits(), e2.to_bits());
                }

                // Check corner vertex coordinates match between adjacent meshes across seam.
                if !nb.crossed_face {
                    let (a_idx, b_idx) = match edge {
                        TileEdge::Right => (n, 0),
                        TileEdge::Left => (0, n),
                        TileEdge::Top => (n * stride, 0),
                        TileEdge::Bottom => (0, n * stride),
                    };
                    let pa = mesh.positions_ecef[a_idx];
                    let pb = nb_mesh.positions_ecef[b_idx];
                    assert!(
                        pa.distance_to(pb) < 1e-6,
                        "seam vertex mismatch on {tile:?} edge {edge:?}: {}",
                        pa.distance_to(pb)
                    );
                }
            }
        }
    }
}

#[test]
fn three_face_corners_are_consistent() {
    let world = TerrainWorld::earth_default();
    let corner_signs = [-1.0_f64, 1.0];
    let inv_sqrt3 = 1.0 / 3.0_f64.sqrt();
    for sx in corner_signs {
        for sy in corner_signs {
            for sz in corner_signs {
                let corner_unit =
                    EcefMeters::from_raw(sx * inv_sqrt3, sy * inv_sqrt3, sz * inv_sqrt3);
                let x_face = if sx > 0.0 {
                    CubeFace::PosX
                } else {
                    CubeFace::NegX
                };
                let y_face = if sy > 0.0 {
                    CubeFace::PosY
                } else {
                    CubeFace::NegY
                };
                let z_face = if sz > 0.0 {
                    CubeFace::PosZ
                } else {
                    CubeFace::NegZ
                };

                let e_direct = world.sample_elevation(corner_unit);
                for face in [x_face, y_face, z_face] {
                    let mut matched = false;
                    for u in [0.0_f64, 1.0] {
                        for v in [0.0_f64, 1.0] {
                            let cand = face_uv_to_unit(face, strata_globe::FaceUv::clamped(u, v));
                            if cand.distance_to(corner_unit) < 1e-12 {
                                let e_face = world.sample_elevation(cand);
                                assert!(
                                    (e_direct - e_face).abs() < 1e-9,
                                    "corner elevation discrepancy on {face:?}: {e_direct} vs {e_face}"
                                );
                                matched = true;
                            }
                        }
                    }
                    assert!(matched, "expected corner on {face:?}");
                }
            }
        }
    }
}

#[test]
fn tile_range_is_strictly_conservative_across_lods_and_tight_on_ocean() {
    let world = TerrainWorld::earth_default();
    let planet = world.planet();
    let global = world.global_range();
    assert!(global.span_meters() > 8_000.0);

    // Verify thousands of points across LOD 0..=12.
    let mut checked = 0_usize;
    for lod in 0_u8..=12 {
        let dim = 1_u32 << lod;
        let step = (dim / 4).max(1);
        for face in CubeFace::ALL {
            let mut ty = 0_u32;
            while ty < dim {
                let mut tx = 0_u32;
                while tx < dim {
                    let tile = TileId {
                        face,
                        lod,
                        x: tx,
                        y: ty,
                    };
                    let range = world.tile_range(planet, tile);
                    assert!(range.min_meters >= global.min_meters - 1e-6);
                    assert!(range.max_meters <= global.max_meters + 1e-6);

                    for sy in 0..=4 {
                        let v = f64::from(sy) * 0.25;
                        for sx in 0..=4 {
                            let u = f64::from(sx) * 0.25;
                            let unit = face_uv_to_unit(
                                face,
                                tile.tile_uv_to_face_uv(TileUv::clamped(u, v)),
                            );
                            let elev = world.sample_elevation(unit);
                            assert!(
                                elev >= range.min_meters && elev <= range.max_meters,
                                "tile {tile:?} range [{}, {}] failed to bound elev {elev} at ({u},{v})",
                                range.min_meters,
                                range.max_meters
                            );
                            checked += 1;
                        }
                    }
                    tx = tx.saturating_add(step);
                    if lod > 4 && tx > step * 2 {
                        break;
                    }
                }
                ty = ty.saturating_add(step);
                if lod > 4 && ty > step * 2 {
                    break;
                }
            }
        }
    }
    assert!(
        checked > 5_000,
        "expected >5000 checked samples, got {checked}"
    );

    // Verify flat mid-Pacific ocean tile at LOD 7 has a much tighter range than global_range.
    let pacific = lat_lon_to_unit(0.0, -140.0);
    let Ok((face, face_uv)) = strata_globe::ecef_to_face_uv(pacific) else {
        unreachable!("unit vector");
    };
    let (tx, ty, _) = strata_globe::face_uv_to_tile_uv(7, face_uv);
    let ocean_tile = TileId {
        face,
        lod: 7,
        x: tx,
        y: ty,
    };
    let ocean_range = world.tile_range(planet, ocean_tile);
    assert!(
        ocean_range.span_meters() < global.span_meters() * 0.12,
        "expected tight ocean tile range, got {} vs global {}",
        ocean_range.span_meters(),
        global.span_meters()
    );
    assert!(ocean_range.max_meters < -1_500.0);
}

#[test]
fn extreme_landmarks_tile_range_is_conservative() {
    let world = TerrainWorld::earth_default();
    let planet = world.planet();

    for (label, lat, lon) in [
        ("Himalayas", 28.0, 86.9),
        ("Mariana", 11.3, 142.2),
        ("Alps", 46.5, 8.0),
        ("Andes", -15.0, -70.0),
        ("NorthPole", 89.8, 120.0),
        ("SouthPole", -89.8, -45.0),
    ] {
        let center = lat_lon_to_unit(lat, lon);
        let Ok((f, fuv)) = strata_globe::ecef_to_face_uv(center) else {
            unreachable!("unit vector");
        };
        for lod in [4_u8, 7, 10] {
            let (tx, ty, _) = strata_globe::face_uv_to_tile_uv(lod, fuv);
            let tile = TileId {
                face: f,
                lod,
                x: tx,
                y: ty,
            };
            let range = world.tile_range(planet, tile);
            for sy in 0..=8 {
                let v = f64::from(sy) * 0.125;
                for sx in 0..=8 {
                    let u = f64::from(sx) * 0.125;
                    let unit = face_uv_to_unit(f, tile.tile_uv_to_face_uv(TileUv::clamped(u, v)));
                    let elev = world.sample_elevation(unit);
                    assert!(
                        elev >= range.min_meters && elev <= range.max_meters,
                        "{label} {tile:?} range [{}, {}] failed to bound elev {elev} at ({u},{v})",
                        range.min_meters,
                        range.max_meters
                    );
                }
            }
        }
    }
}
