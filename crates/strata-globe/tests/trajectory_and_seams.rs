use std::collections::{HashMap, HashSet};
use std::error::Error;

use strata_globe::{
    CameraViewF64, CubeFace, EcefMeters, FaceUv, LocalOrigin, LodConfig, PlanetSpec, RebasePolicy,
    RenderVec3, TileEdge, TileId, build_tile_mesh_f64, face_uv_to_unit, select_tiles,
};

#[derive(Clone, Copy)]
struct Vec3F32 {
    x: f32,
    y: f32,
    z: f32,
}

impl Vec3F32 {
    fn from_render(v: RenderVec3) -> Self {
        Self {
            x: v.x,
            y: v.y,
            z: v.z,
        }
    }

    fn sub(self, o: Self) -> Self {
        Self {
            x: self.x - o.x,
            y: self.y - o.y,
            z: self.z - o.z,
        }
    }

    fn dot(self, o: Self) -> f32 {
        self.x * o.x + self.y * o.y + self.z * o.z
    }

    fn cross(self, o: Self) -> Self {
        Self {
            x: self.y * o.z - self.z * o.y,
            y: self.z * o.x - self.x * o.z,
            z: self.x * o.y - self.y * o.x,
        }
    }

    fn normalized(self) -> Self {
        let len = (self.x * self.x + self.y * self.y + self.z * self.z).sqrt();
        Self {
            x: self.x / len,
            y: self.y / len,
            z: self.z / len,
        }
    }
}

fn project_f64_to_pixels(
    cam: &CameraViewF64,
    point: EcefMeters,
    width_px: f64,
    height_px: f64,
) -> Option<(f64, f64)> {
    let rel = point - cam.position;
    let z = rel.dot(cam.forward);
    if z <= 0.05 {
        return None;
    }
    let x = rel.dot(cam.right);
    let y = rel.dot(cam.up);
    let half_tan_y = (cam.fov_y_rad * 0.5).tan();
    let half_tan_x = half_tan_y * cam.aspect;
    let ndc_x = x / (z * half_tan_x);
    let ndc_y = y / (z * half_tan_y);
    Some((
        (ndc_x + 1.0) * 0.5 * width_px,
        (1.0 - ndc_y) * 0.5 * height_px,
    ))
}

#[expect(
    clippy::cast_possible_truncation,
    reason = "intentional f64->f32 projection comparison simulating GPU f32 camera pipeline"
)]
fn project_f32_to_pixels(
    origin: LocalOrigin,
    cam: &CameraViewF64,
    target: EcefMeters,
    point: EcefMeters,
    width_px: f32,
) -> Option<(f32, f32)> {
    let cam_rel = Vec3F32::from_render(origin.to_render_vec3(cam.position));
    let tgt_rel = Vec3F32::from_render(origin.to_render_vec3(target));
    let pt_rel = Vec3F32::from_render(origin.to_render_vec3(point));
    let up_f32 = Vec3F32 {
        x: cam.up.x as f32,
        y: cam.up.y as f32,
        z: cam.up.z as f32,
    };

    let forward = tgt_rel.sub(cam_rel).normalized();
    let right = forward.cross(up_f32).normalized();
    let up = right.cross(forward).normalized();

    let rel = pt_rel.sub(cam_rel);
    let z = rel.dot(forward);
    if z <= 0.05 {
        return None;
    }
    let x = rel.dot(right);
    let y = rel.dot(up);
    let half_tan_y = (cam.fov_y_rad as f32 * 0.5).tan();
    let half_tan_x = half_tan_y * (cam.aspect as f32);
    let ndc_x = x / (z * half_tan_x);
    let ndc_y = y / (z * half_tan_y);
    let height_px = cam.viewport_height_px as f32;
    Some((
        (ndc_x + 1.0) * 0.5 * width_px,
        (1.0 - ndc_y) * 0.5 * height_px,
    ))
}

#[test]
fn logarithmic_zoom_trajectory_meets_precision_budgets() -> Result<(), Box<dyn Error>> {
    let planet = PlanetSpec::earth();
    let r = planet.radius_meters();
    let fov_y = 60.0_f64.to_radians();
    let width = 1920.0;
    let height = 1080.0;
    let aspect = width / height;

    // Anchor targets at: interior, intra-face seam, 2-face cube edge, 3-face cube corner.
    let targets = [
        face_uv_to_unit(CubeFace::PosZ, FaceUv::clamped(0.37, 0.61)).scale(r),
        face_uv_to_unit(CubeFace::PosZ, FaceUv::clamped(0.5, 0.35)).scale(r),
        face_uv_to_unit(CubeFace::PosX, FaceUv::clamped(0.0, 0.5)).scale(r),
        face_uv_to_unit(CubeFace::PosX, FaceUv::clamped(0.0, 1.0)).scale(r),
    ];

    let lod_cfg = LodConfig::standard();
    let rebase_policy = RebasePolicy::standard();

    let mut max_local_pos_err_m = 0.0_f64;
    let mut max_pixel_drift = 0.0_f64;
    let mut total_rebases = 0_u32;

    for target_ecef in targets {
        let up_normal = target_ecef.normalized()?;
        let east_hint = if up_normal.y.abs() < 0.9 {
            EcefMeters::from_raw(0.0, 1.0, 0.0)
                .cross(up_normal)
                .normalized()?
        } else {
            EcefMeters::from_raw(0.0, 0.0, 1.0)
        };
        let north_up = up_normal.cross(east_hint).normalized()?;

        let mut origin = LocalOrigin::new(target_ecef + up_normal.scale(20_000_000.0))?;
        let mut previously_split = HashSet::new();

        // Two full cycles: 20_000 km -> 20 m -> 20_000 km.
        for cycle in 0..2_u32 {
            let steps = 40_u32;
            for step in 0..=steps {
                let t_raw = f64::from(step) / f64::from(steps);
                let t = if cycle % 2 == 0 { t_raw } else { 1.0 - t_raw };
                let alt = 20_000_000.0_f64.powf(1.0 - t) * 20.0_f64.powf(t);
                let lateral = (alt * 0.2).min(5_000.0);
                let cam_pos = target_ecef + up_normal.scale(alt) + east_hint.scale(lateral);

                if rebase_policy.maybe_rebase(&mut origin, cam_pos, alt) {
                    total_rebases += 1;
                }

                let nearby = target_ecef + north_up.scale(1_500.0);
                if cam_pos.distance_to(nearby) <= 10_000.0 {
                    let rel = origin.to_render_vec3(nearby);
                    let err_m = origin.reconstruct_ecef_f64(rel).distance_to(nearby);
                    max_local_pos_err_m = max_local_pos_err_m.max(err_m);
                    assert!(err_m <= 1e-3, "local f32 error {err_m:.6e} m > 1 mm");
                }

                let cam_view =
                    CameraViewF64::look_at(cam_pos, target_ecef, north_up, fov_y, aspect, height)?;
                let selection = select_tiles(planet, &cam_view, lod_cfg, &mut previously_split);
                assert!(!selection.tiles.is_empty());
                verify_quadtree_2_to_1_balance(&selection.tiles);

                let px64 = project_f64_to_pixels(&cam_view, target_ecef, width, height);
                #[expect(
                    clippy::cast_possible_truncation,
                    reason = "test pixel width fits exactly in f32"
                )]
                let px32 = project_f32_to_pixels(
                    origin,
                    &cam_view,
                    target_ecef,
                    target_ecef,
                    width as f32,
                );
                if let (Some((x64, y64)), Some((x32, y32))) = (px64, px32) {
                    let drift = (x64 - f64::from(x32)).hypot(y64 - f64::from(y32));
                    max_pixel_drift = max_pixel_drift.max(drift);
                    assert!(drift < 0.5, "screen drift {drift:.4} px >= 0.5 px");
                }
            }
        }
    }

    assert!(total_rebases > 0);
    assert!(max_local_pos_err_m <= 1e-3);
    assert!(max_pixel_drift < 0.5);
    println!(
        "METRIC trajectory_local_err_m={max_local_pos_err_m:.3e} max_pixel_drift_px={max_pixel_drift:.4e} rebases={total_rebases}"
    );
    Ok(())
}

#[test]
fn lateral_seam_and_corner_traverse_preserves_precision_and_balance() -> Result<(), Box<dyn Error>>
{
    let planet = PlanetSpec::earth();
    let r = planet.radius_meters();
    let fov_y = 60.0_f64.to_radians();
    let height = 1080.0;
    let aspect = 1920.0 / height;
    let lod_cfg = LodConfig::standard();
    let rebase_policy = RebasePolicy::standard();

    let corner = face_uv_to_unit(CubeFace::PosX, FaceUv::clamped(0.0, 1.0)).scale(r);
    let corner_up = corner.normalized()?;
    let mut origin = LocalOrigin::new(corner + corner_up.scale(20.0))?;
    let mut previously_split = HashSet::new();
    let mut total_rebases = 0_u32;

    for step in -20_i32..=20_i32 {
        let offset_m = f64::from(step) * 250.0;
        let ground = (corner + EcefMeters::from_raw(offset_m, -offset_m * 0.5, -offset_m * 0.2))
            .normalized()?
            .scale(r);
        let up = ground.normalized()?;
        let cam_pos = ground + up.scale(20.0);
        if rebase_policy.maybe_rebase(&mut origin, cam_pos, 20.0) {
            total_rebases += 1;
        }
        let look_target = ground + EcefMeters::from_raw(5.0, 0.0, 0.0);
        let cam_view = CameraViewF64::look_at(cam_pos, look_target, up, fov_y, aspect, height)?;
        let selection = select_tiles(planet, &cam_view, lod_cfg, &mut previously_split);
        verify_quadtree_2_to_1_balance(&selection.tiles);

        if let Some(finest) = selection.tiles.iter().max_by_key(|t| t.id.lod) {
            let mesh = build_tile_mesh_f64(planet, finest.id, finest.coarser_edges);
            let render = mesh.to_render_mesh(origin);
            for (v64, v32) in mesh.positions_ecef.iter().zip(render.positions.iter()) {
                if cam_pos.distance_to(*v64) <= 10_000.0 {
                    let rec = origin.reconstruct_ecef_f64(RenderVec3::new(v32[0], v32[1], v32[2]));
                    assert!(rec.distance_to(*v64) <= 1e-3);
                }
            }
        }
    }

    assert!(total_rebases > 0);
    Ok(())
}

#[test]
fn lod_hysteresis_prevents_split_merge_chatter() -> Result<(), Box<dyn Error>> {
    let planet = PlanetSpec::earth();
    let r = planet.radius_meters();
    let target = face_uv_to_unit(CubeFace::PosZ, FaceUv::clamped(0.5, 0.5)).scale(r);
    let up = target.normalized()?;
    let north = EcefMeters::from_raw(0.0, 1.0, 0.0);

    // 1. Pure LOD hysteresis: disable horizon/frustum boundary clipping, refine at cam_in,
    // settle at the outer oscillation extremum cam_b, then oscillate cam_a <-> cam_b.
    // Even though cam_a is closer, no tile chatters (re-splits and re-merges).
    let config_pure_lod = LodConfig {
        enable_horizon_culling: false,
        enable_frustum_culling: false,
        ..LodConfig::standard()
    };
    let mut split_state = HashSet::new();
    let mid_alt = 120_000.0;
    let cam_in = CameraViewF64::look_at(
        target + up.scale(mid_alt * 0.85),
        target,
        north,
        60.0_f64.to_radians(),
        16.0 / 9.0,
        1080.0,
    )?;
    let _ = select_tiles(planet, &cam_in, config_pure_lod, &mut split_state);

    let cam_a = CameraViewF64::look_at(
        target + up.scale(mid_alt),
        target,
        north,
        60.0_f64.to_radians(),
        16.0 / 9.0,
        1080.0,
    )?;
    let cam_b = CameraViewF64::look_at(
        target + up.scale(mid_alt * 1.05),
        target,
        north,
        60.0_f64.to_radians(),
        16.0 / 9.0,
        1080.0,
    )?;

    let settled_b = select_tiles(planet, &cam_b, config_pure_lod, &mut split_state);
    let settled_ids: Vec<TileId> = settled_b.tiles.iter().map(|t| t.id).collect();

    for _ in 0..4 {
        let sel_a = select_tiles(planet, &cam_a, config_pure_lod, &mut split_state);
        let ids_a: Vec<TileId> = sel_a.tiles.iter().map(|t| t.id).collect();
        assert_eq!(ids_a, settled_ids);

        let sel_b = select_tiles(planet, &cam_b, config_pure_lod, &mut split_state);
        let ids_b: Vec<TileId> = sel_b.tiles.iter().map(|t| t.id).collect();
        assert_eq!(ids_b, settled_ids);
    }

    // 2. With full horizon + frustum culling enabled, peak central LOD stays stable across jitter.
    let std_cfg = LodConfig::standard();
    let mut std_split = HashSet::new();
    let _ = select_tiles(planet, &cam_in, std_cfg, &mut std_split);
    let sel_std_b = select_tiles(planet, &cam_b, std_cfg, &mut std_split);
    let sel_std_a = select_tiles(planet, &cam_a, std_cfg, &mut std_split);
    assert_eq!(sel_std_a.max_lod_selected, sel_std_b.max_lod_selected);
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
            // Check parent (coarser by 1) — must not have grandparent (coarser by 2+) as leaf.
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
