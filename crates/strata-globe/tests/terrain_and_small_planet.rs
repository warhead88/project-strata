use std::collections::{HashMap, HashSet};
use std::error::Error;

use strata_globe::{
    CameraViewF64, CoarserEdges, CubeFace, DEFAULT_GRID_SEGMENTS, EcefMeters, ElevationRange,
    ElevationSampler, FaceUv, GlobeError, LocalOrigin, LodConfig, PlanetSpec, RenderVec3,
    SurfaceCoord, TileEdge, TileId, build_tile_mesh_f64_with_elevation, ecef_to_tile_uv,
    face_uv_to_unit, select_tiles_with_elevation,
};

struct MountainOceanSampler {
    range: ElevationRange,
}

impl MountainOceanSampler {
    fn earth_relief() -> Result<Self, GlobeError> {
        Ok(Self {
            range: ElevationRange::new(-10_000.0, 9_000.0)?,
        })
    }

    fn small_planet_relief() -> Result<Self, GlobeError> {
        Ok(Self {
            range: ElevationRange::new(-2_000.0, 2_000.0)?,
        })
    }
}

impl ElevationSampler for MountainOceanSampler {
    fn sample_elevation(&self, u: EcefMeters) -> f64 {
        let wave = (u.x * 7.0 + u.y * 5.0).sin() * (u.z * 9.0 - u.x * 3.0).cos();
        let ridge = (u.y * 13.0 - u.z * 11.0).sin();
        let norm_val = (0.65 * wave + 0.35 * ridge).clamp(-1.0, 1.0);
        let mid = 0.5 * (self.range.min_meters + self.range.max_meters);
        let half = 0.5 * self.range.span_meters();
        mid + half * norm_val
    }

    fn global_range(&self) -> ElevationRange {
        self.range
    }
}

struct HorizonPeakSampler {
    peak_dir: EcefMeters,
    peak_height_m: f64,
    range: ElevationRange,
}

impl ElevationSampler for HorizonPeakSampler {
    fn sample_elevation(&self, unit_direction: EcefMeters) -> f64 {
        let angle = unit_direction.dot(self.peak_dir).clamp(-1.0, 1.0).acos();
        let falloff = (-(angle / 0.03).powi(2)).exp();
        self.peak_height_m * falloff
    }

    fn global_range(&self) -> ElevationRange {
        self.range
    }
}

#[test]
fn elevation_range_validates_and_clamps_extreme_depth() -> Result<(), GlobeError> {
    assert!(ElevationRange::FLAT.is_flat());
    assert!(ElevationRange::FLAT.span_meters().abs() <= 1e-12);
    assert!(ElevationRange::new(100.0, 50.0).is_err());
    assert!(ElevationRange::new(f64::NAN, 100.0).is_err());
    assert!(ElevationRange::new(-10.0, f64::INFINITY).is_err());

    let planet = PlanetSpec::new(200_000.0, 14, DEFAULT_GRID_SEGMENTS)?;
    let extreme = ElevationRange::new(-500_000.0, 2_000.0)?;
    let clamped = extreme.clamp_planet_radius(planet);
    assert!(planet.radius_meters() + clamped.min_meters >= 1.0);
    assert!(extreme.min_radius_meters(planet) >= 1.0);
    assert!((extreme.max_radius_meters(planet) - 202_000.0).abs() <= 1e-9);
    Ok(())
}

#[test]
fn terrain_meshes_stay_continuous_across_edges_and_corners() -> Result<(), Box<dyn Error>> {
    let planet = PlanetSpec::earth();
    let sampler = MountainOceanSampler::earth_relief()?;
    let n = planet.grid_segments() as usize;
    let stride = n + 1;

    // 2-face seam between +X (u=0) and +Z (u=1) at LOD 4, plus 3-face corner (+X/+Y/+Z).
    let lod = 4_u8;
    let max_idx = (1_u32 << lod) - 1;
    let east_tile = TileId::new(CubeFace::PosX, lod, 0, max_idx)?;
    let front_tile = TileId::new(CubeFace::PosZ, lod, max_idx, max_idx)?;
    let north_tile = TileId::new(CubeFace::PosY, lod, max_idx, 0)?;

    let east_mesh =
        build_tile_mesh_f64_with_elevation(planet, east_tile, CoarserEdges::NONE, &sampler);
    let front_mesh =
        build_tile_mesh_f64_with_elevation(planet, front_tile, CoarserEdges::NONE, &sampler);
    let north_mesh =
        build_tile_mesh_f64_with_elevation(planet, north_tile, CoarserEdges::NONE, &sampler);

    let corner_ecef = east_mesh.positions_ecef[n * stride];
    let origin = LocalOrigin::new(corner_ecef)?;
    let east_render = east_mesh.to_render_mesh(origin);
    let front_render = front_mesh.to_render_mesh(origin);
    let north_render = north_mesh.to_render_mesh(origin);

    let mut non_radial_normals = 0_usize;
    for iy in 0..=n {
        let east_idx = iy * stride;
        let front_idx = iy * stride + n;
        let east_pos = east_mesh.positions_ecef[east_idx];
        let front_pos = front_mesh.positions_ecef[front_idx];
        assert!(east_pos.distance_to(front_pos) <= 1e-6);

        let dx = east_render.positions[east_idx][0] - front_render.positions[front_idx][0];
        let dy = east_render.positions[east_idx][1] - front_render.positions[front_idx][1];
        let dz = east_render.positions[east_idx][2] - front_render.positions[front_idx][2];
        let render_dist = (dx * dx + dy * dy + dz * dz).sqrt();
        assert!(render_dist <= 1e-3);

        let unit = east_pos.normalized()?;
        let east_nrm = EcefMeters::from_raw(
            f64::from(east_mesh.normals[east_idx][0]),
            f64::from(east_mesh.normals[east_idx][1]),
            f64::from(east_mesh.normals[east_idx][2]),
        );
        let front_nrm = EcefMeters::from_raw(
            f64::from(front_mesh.normals[front_idx][0]),
            f64::from(front_mesh.normals[front_idx][1]),
            f64::from(front_mesh.normals[front_idx][2]),
        );
        assert!((east_nrm.norm() - 1.0).abs() <= 1e-5);
        assert!(east_nrm.dot(unit) > 0.0);
        assert!(east_nrm.distance_to(front_nrm) <= 1e-5);
        if east_nrm.distance_to(unit) > 1e-4 {
            non_radial_normals += 1;
        }
    }
    assert!(
        non_radial_normals > 0,
        "slope normals should tilt away from radial"
    );

    // 3-face corner (+X/+Y/+Z) vertex continuity & outward normals across all 3 tiles.
    let corner_east = east_mesh.positions_ecef[n * stride];
    let corner_front = front_mesh.positions_ecef[n * stride + n];
    let corner_north = north_mesh.positions_ecef[n];
    assert!(corner_east.distance_to(corner_front) <= 1e-6);
    assert!(corner_east.distance_to(corner_north) <= 1e-6);
    assert_eq!(
        east_render.positions[n * stride].map(f32::to_bits),
        front_render.positions[n * stride + n].map(f32::to_bits)
    );
    assert_eq!(
        east_render.positions[n * stride].map(f32::to_bits),
        north_render.positions[n].map(f32::to_bits)
    );
    Ok(())
}

#[test]
fn terrain_stitching_across_all_coarser_masks_is_watertight() -> Result<(), GlobeError> {
    let planet = PlanetSpec::earth();
    let sampler = MountainOceanSampler::earth_relief()?;
    let n = planet.grid_segments();
    let stride = n + 1;

    let fine_tile = TileId::new(CubeFace::PosX, 4, 4, 4)?;
    let coarse_parent =
        fine_tile
            .parent()
            .ok_or(GlobeError::InvalidTileCoord { lod: 4, x: 4, y: 4 })?;
    let coarse_mesh =
        build_tile_mesh_f64_with_elevation(planet, coarse_parent, CoarserEdges::NONE, &sampler);

    for mask in 0_u8..16 {
        let coarser = CoarserEdges::from_mask(mask);
        let fine_mesh = build_tile_mesh_f64_with_elevation(planet, fine_tile, coarser, &sampler);
        let referenced: HashSet<u32> = fine_mesh.indices.iter().copied().collect();

        for i in 0..=n {
            if i.is_multiple_of(2) {
                // Fine bottom edge (v=0) lies along coarse_parent's bottom edge at x/2 = 0..=n/2.
                let fine_idx = i as usize;
                let coarse_idx = (i / 2) as usize;
                assert!(
                    fine_mesh.positions_ecef[fine_idx]
                        .distance_to(coarse_mesh.positions_ecef[coarse_idx])
                        <= 1e-12
                );
            } else {
                assert_eq!(!coarser.bottom(), referenced.contains(&i));
                assert_eq!(!coarser.right(), referenced.contains(&(i * stride + n)));
                assert_eq!(!coarser.top(), referenced.contains(&(n * stride + i)));
                assert_eq!(!coarser.left(), referenced.contains(&(i * stride)));
            }
        }
    }
    Ok(())
}

#[test]
fn horizon_culling_keeps_visible_peak_beyond_geometric_horizon() -> Result<(), GlobeError> {
    let planet = PlanetSpec::earth();
    let r = planet.radius_meters();
    let cam_alt = 50_000.0;
    let cam_r = r + cam_alt;
    let geom_horizon_angle = (r / cam_r).acos();
    // Place a 9_000 m peak slightly beyond the smooth geometric sphere horizon.
    let peak_angle = geom_horizon_angle + 0.025;
    let peak_dir = EcefMeters::from_raw(peak_angle.sin(), 0.0, peak_angle.cos()).normalized()?;

    let sampler = HorizonPeakSampler {
        peak_dir,
        peak_height_m: 9_000.0,
        range: ElevationRange::new(0.0, 9_000.0)?,
    };

    let cam_pos = EcefMeters::from_raw(0.0, 0.0, cam_r);
    let peak_pos = peak_dir.scale(r + 9_000.0);
    let cam_view = CameraViewF64::look_at(
        cam_pos,
        peak_pos,
        EcefMeters::from_raw(0.0, 1.0, 0.0),
        60.0_f64.to_radians(),
        16.0 / 9.0,
        1080.0,
    )?;

    let mut split_state = HashSet::new();
    let sel = select_tiles_with_elevation(
        planet,
        &cam_view,
        LodConfig::standard(),
        &mut split_state,
        &sampler,
    )?;

    let (peak_face, peak_uv) = strata_globe::ecef_to_face_uv(peak_dir)?;
    let contains_peak = sel.tiles.iter().any(|t| {
        if t.id.face != peak_face {
            return false;
        }
        let scale = f64::from(1_u32 << t.id.lod);
        let u0 = f64::from(t.id.x) / scale;
        let u1 = f64::from(t.id.x + 1) / scale;
        let v0 = f64::from(t.id.y) / scale;
        let v1 = f64::from(t.id.y + 1) / scale;
        peak_uv.u >= u0 - 1e-9
            && peak_uv.u <= u1 + 1e-9
            && peak_uv.v >= v0 - 1e-9
            && peak_uv.v <= v1 + 1e-9
    });
    assert!(
        contains_peak,
        "tile containing +9000 m peak past smooth horizon must not be culled"
    );
    // Back-side face (-Z) must still be horizon-culled.
    assert!(!sel.tiles.iter().any(|t| t.id.face == CubeFace::NegZ));
    Ok(())
}

#[test]
fn small_planet_with_terrain_preserves_roundtrip_balance_and_seams() -> Result<(), Box<dyn Error>> {
    let planet = PlanetSpec::new(200_000.0, 14, DEFAULT_GRID_SEGMENTS)?;
    let sampler = MountainOceanSampler::small_planet_relief()?;
    let r = planet.radius_meters();

    // 1. SurfaceCoord & ecef_to_tile_uv round-trip across faces and poles.
    for dir in [
        EcefMeters::from_raw(1.0, 0.25, -0.4),
        EcefMeters::from_raw(0.0, 1.0, 0.001),
        EcefMeters::from_raw(-1.0, -1.0, 1.0),
    ] {
        let unit = dir.normalized()?;
        let surf = SurfaceCoord::from_ecef(planet, unit.scale(r + 1_250.0))?;
        let recon = surf.to_ecef(planet)?;
        assert!(recon.distance_to(unit.scale(r + 1_250.0)) <= 1e-6);

        let (tile, tile_uv, _) = ecef_to_tile_uv(planet, unit.scale(r), 10)?;
        let face_uv = tile.tile_uv_to_face_uv(tile_uv);
        let round_unit = face_uv_to_unit(tile.face, face_uv);
        assert!(round_unit.scale(r).distance_to(unit.scale(r)) <= 1e-6);
    }

    // 2. LOD selection clamps min_lod > planet.max_lod() and maintains 2:1 balance.
    let corner_dir = face_uv_to_unit(CubeFace::PosX, FaceUv::clamped(0.0, 1.0));
    let ground = corner_dir.scale(r + sampler.sample_elevation(corner_dir));
    let cam_pos = corner_dir.scale(r + sampler.sample_elevation(corner_dir) + 800.0);
    let cam_view = CameraViewF64::look_at(
        cam_pos,
        ground,
        EcefMeters::from_raw(0.0, 1.0, 0.0),
        60.0_f64.to_radians(),
        16.0 / 9.0,
        1080.0,
    )?;

    let cfg = LodConfig {
        min_lod: 20, // Exceeds planet.max_lod() = 14
        ..LodConfig::standard()
    };
    let mut split_state = HashSet::new();
    let sel = select_tiles_with_elevation(planet, &cam_view, cfg, &mut split_state, &sampler)?;
    assert!(!sel.tiles.is_empty());
    assert!(sel.max_lod_selected <= planet.max_lod());
    verify_quadtree_2_to_1_balance(&sel.tiles);

    // 3. Reconstruct render mesh near small-planet ground within <= 1e-3 m.
    let origin = LocalOrigin::new(cam_pos)?;
    if let Some(finest) = sel.tiles.iter().max_by_key(|t| t.id.lod) {
        let mesh =
            build_tile_mesh_f64_with_elevation(planet, finest.id, finest.coarser_edges, &sampler);
        let render = mesh.to_render_mesh(origin);
        for (v64, v32) in mesh.positions_ecef.iter().zip(render.positions.iter()) {
            let rec = origin.reconstruct_ecef_f64(RenderVec3::new(v32[0], v32[1], v32[2]));
            assert!(rec.distance_to(*v64) <= 1e-3);
        }
    }
    Ok(())
}

fn verify_quadtree_2_to_1_balance(tiles: &[strata_globe::SelectedTile]) {
    let lod_by_id: HashMap<TileId, u8> = tiles.iter().map(|t| (t.id, t.id.lod)).collect();
    for tile in tiles {
        let id = tile.id;
        for edge in TileEdge::ALL {
            let same_lod_nb = id.neighbor(edge).tile;
            if lod_by_id.contains_key(&same_lod_nb) {
                continue;
            }
            let mut cursor = same_lod_nb.parent();
            let mut depth_diff = 1_u8;
            while let Some(anc) = cursor {
                if lod_by_id.contains_key(&anc) {
                    assert!(
                        depth_diff <= 1,
                        "2:1 balance violated: {:?} (LOD {}) borders {:?} (LOD {})",
                        id,
                        id.lod,
                        anc,
                        anc.lod
                    );
                    break;
                }
                cursor = anc.parent();
                depth_diff += 1;
            }
        }
    }
}
