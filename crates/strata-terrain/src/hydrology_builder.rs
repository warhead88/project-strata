use strata_globe::{
    CubeFace, EARTH_MEAN_RADIUS_METERS, EcefMeters, FaceUv, PlanetSpec, face_uv_to_unit,
};

use crate::earth_geography::{TerrainSource, lat_lon_to_unit};
use crate::error::TerrainError;
use crate::hydrology::{LakeDef, LakeId, Navigability, RiverDef, RiverId, RiverWaypoint, Salinity};
use crate::hydrology_data::{EARTH_LAKES, EARTH_RIVERS};
use crate::hydrology_index::{
    BUCKET_DIM, BucketRange, HydrologyIndex, RiverSegRef, TOTAL_BUCKETS, arc_distance_rad,
};
use crate::macro_climate::{angular_distance, evaluate_procedural_macro};

pub(crate) fn build_hydrology_index(
    planet: PlanetSpec,
    source: &TerrainSource,
) -> Result<HydrologyIndex, TerrainError> {
    let radius_m = planet.radius_meters();
    let scale = (radius_m / EARTH_MEAN_RADIUS_METERS).clamp(0.02, 4.0);
    let (lakes, rivers) = match source {
        TerrainSource::Earth { .. } => build_earth_hydrology(scale)?,
        TerrainSource::Procedural { seed } => build_procedural_hydrology(radius_m, *seed)?,
        TerrainSource::Dem { .. } => (Vec::new(), Vec::new()),
    };

    let mut buckets = Vec::with_capacity(TOTAL_BUCKETS);
    let mut bucket_lakes = Vec::new();
    let mut bucket_segs = Vec::new();

    for face in CubeFace::ALL {
        for ty in 0..BUCKET_DIM {
            for tx in 0..BUCKET_DIM {
                let (tile_center, tile_bound_rad) = lod4_tile_cap(face, tx, ty);
                let lake_start = u32::try_from(bucket_lakes.len()).unwrap_or(u32::MAX);
                for (li, lake) in lakes.iter().enumerate() {
                    if angular_distance(tile_center, lake.center_unit)
                        <= tile_bound_rad + lake.radius_rad * 1.45
                        && let Ok(idx) = u16::try_from(li)
                    {
                        bucket_lakes.push(idx);
                    }
                }
                let lake_len =
                    u16::try_from(bucket_lakes.len().saturating_sub(lake_start as usize))
                        .unwrap_or(u16::MAX);

                let seg_start = u32::try_from(bucket_segs.len()).unwrap_or(u32::MAX);
                for (ri, river) in rivers.iter().enumerate() {
                    for (si, pair) in river.waypoints.windows(2).enumerate() {
                        let valley_rad =
                            pair[0].valley_half_width_m.max(pair[1].valley_half_width_m) / radius_m;
                        if arc_distance_rad(tile_center, pair[0].unit_dir, pair[1].unit_dir)
                            <= tile_bound_rad + valley_rad * 1.3
                            && let (Ok(r_idx), Ok(s_idx)) = (u16::try_from(ri), u16::try_from(si))
                        {
                            bucket_segs.push(RiverSegRef {
                                river_idx: r_idx,
                                seg_idx: s_idx,
                            });
                        }
                    }
                }
                let seg_len = u16::try_from(bucket_segs.len().saturating_sub(seg_start as usize))
                    .unwrap_or(u16::MAX);

                buckets.push(BucketRange {
                    lake_start,
                    lake_len,
                    seg_start,
                    seg_len,
                });
            }
        }
    }

    Ok(HydrologyIndex::from_parts(
        radius_m,
        lakes,
        rivers,
        buckets,
        bucket_lakes,
        bucket_segs,
    ))
}

fn build_earth_hydrology(scale: f64) -> Result<(Vec<LakeDef>, Vec<RiverDef>), TerrainError> {
    let mut lakes = Vec::with_capacity(EARTH_LAKES.len());
    for (idx, spec) in EARTH_LAKES.iter().enumerate() {
        lakes.push(LakeDef {
            id: LakeId(u32::try_from(idx).unwrap_or(u32::MAX)),
            name: spec.name,
            center_unit: lat_lon_to_unit(spec.lat_deg, spec.lon_deg),
            radius_rad: spec.radius_deg.to_radians(),
            surface_elevation_m: spec.surface_elevation_m,
            max_depth_m: spec.max_depth_m,
            salinity: spec.salinity,
        });
    }

    let mut rivers = Vec::with_capacity(EARTH_RIVERS.len());
    for (idx, spec) in EARTH_RIVERS.iter().enumerate() {
        let wps = spec
            .waypoints
            .iter()
            .map(|w| RiverWaypoint {
                unit_dir: lat_lon_to_unit(w.lat_deg, w.lon_deg),
                surface_elevation_m: w.surface_elevation_m,
                half_width_m: w.half_width_m * scale,
                channel_depth_m: w.channel_depth_m,
                valley_half_width_m: w.valley_half_width_m * scale,
                discharge_m3s: w.discharge_m3s,
            })
            .collect();
        rivers.push(RiverDef::new(
            RiverId(u32::try_from(idx).unwrap_or(u32::MAX)),
            spec.name,
            spec.navigability,
            wps,
        )?);
    }
    Ok((lakes, rivers))
}

fn build_procedural_hydrology(
    planet_radius_m: f64,
    seed: u64,
) -> Result<(Vec<LakeDef>, Vec<RiverDef>), TerrainError> {
    let scale = (planet_radius_m / EARTH_MEAN_RADIUS_METERS).clamp(0.03, 2.0);
    let mut lakes = Vec::new();
    let mut rivers = Vec::new();

    let candidates = 64_u32;
    let golden_angle = std::f64::consts::PI * (3.0 - 5.0_f64.sqrt());
    for i in 0..candidates {
        let y = 1.0 - (f64::from(i) + 0.5) * (2.0 / f64::from(candidates));
        let r = (1.0 - y * y).max(0.0).sqrt();
        #[expect(
            clippy::cast_precision_loss,
            reason = "seed & 0xFFFF is at most 65535, exact in f64"
        )]
        let seed_offset = ((seed & 0xFFFF) as f64) * 0.01;
        let theta = golden_angle * f64::from(i) + seed_offset;
        let dir = EcefMeters::from_raw(r * theta.cos(), y, r * theta.sin());
        let macro_s = evaluate_procedural_macro(dir, seed);
        let elev = f64::from(macro_s.macro_elevation_m);

        if lakes.len() < 6 && elev > 120.0 && elev < 1400.0 && i % 7 == 2 {
            let id = LakeId(u32::try_from(lakes.len()).unwrap_or(u32::MAX));
            lakes.push(LakeDef {
                id,
                name: "Procedural Lake",
                center_unit: dir,
                radius_rad: 0.016,
                surface_elevation_m: elev,
                max_depth_m: 120.0 + f64::from(i % 5) * 45.0,
                salinity: if i % 14 == 2 {
                    Salinity::Saline
                } else {
                    Salinity::Freshwater
                },
            });
        }

        if rivers.len() < 10
            && elev > 420.0
            && let Some(wps) = trace_procedural_river(dir, elev, scale, seed)
        {
            let id = RiverId(u32::try_from(rivers.len()).unwrap_or(u32::MAX));
            rivers.push(RiverDef::new(
                id,
                "Procedural River",
                Navigability::RiverVessel,
                wps,
            )?);
        }
    }

    Ok((lakes, rivers))
}

fn trace_procedural_river(
    start: EcefMeters,
    start_elev: f64,
    scale: f64,
    seed: u64,
) -> Option<Vec<RiverWaypoint>> {
    let step_rad = 0.045_f64;
    let mut cur = start;
    let mut cur_elev = start_elev;
    let mut wps = vec![RiverWaypoint {
        unit_dir: cur,
        surface_elevation_m: cur_elev,
        half_width_m: 400.0 * scale,
        channel_depth_m: 5.0,
        valley_half_width_m: 14_000.0 * scale,
        discharge_m3s: 600.0,
    }];

    for step in 1..=10 {
        let up = if cur.y.abs() > 0.98 {
            EcefMeters::from_raw(0.0, 0.0, 1.0)
        } else {
            EcefMeters::from_raw(0.0, 1.0, 0.0)
        };
        let t1_raw = up.cross(cur);
        let t1 = t1_raw.scale(1.0 / t1_raw.norm());
        let t2 = cur.cross(t1);

        let mut best_dir = cur;
        let mut best_elev = f64::INFINITY;
        for k in 0..8 {
            let ang = std::f64::consts::TAU * (f64::from(k) / 8.0);
            let cand_raw = cur + (t1.scale(ang.cos()) + t2.scale(ang.sin())).scale(step_rad);
            let cand = cand_raw.scale(1.0 / cand_raw.norm());
            let e = f64::from(evaluate_procedural_macro(cand, seed).macro_elevation_m);
            if e < best_elev {
                best_elev = e;
                best_dir = cand;
            }
        }
        cur = best_dir;
        cur_elev = best_elev.min(cur_elev).max(0.0);
        let progress = f64::from(step) / 10.0;
        wps.push(RiverWaypoint {
            unit_dir: cur,
            surface_elevation_m: cur_elev,
            half_width_m: (400.0 + progress * 3_400.0) * scale,
            channel_depth_m: 5.0 + progress * 16.0,
            valley_half_width_m: (14_000.0 + progress * 32_000.0) * scale,
            discharge_m3s: 600.0 + progress * 18_000.0,
        });
        if best_elev <= 0.0 {
            break;
        }
    }

    if wps.len() >= 3 && start_elev - cur_elev > 150.0 {
        Some(wps)
    } else {
        None
    }
}

#[expect(
    clippy::cast_precision_loss,
    reason = "LOD 4 tile coordinates are 0..16, exact in f64"
)]
fn lod4_tile_cap(face: CubeFace, tx: usize, ty: usize) -> (EcefMeters, f64) {
    let inv = 1.0 / (BUCKET_DIM as f64);
    let u0 = (tx as f64) * inv;
    let v0 = (ty as f64) * inv;
    let center = face_uv_to_unit(face, FaceUv::clamped(u0 + 0.5 * inv, v0 + 0.5 * inv));
    let c00 = face_uv_to_unit(face, FaceUv::clamped(u0, v0));
    let c10 = face_uv_to_unit(face, FaceUv::clamped(u0 + inv, v0));
    let c01 = face_uv_to_unit(face, FaceUv::clamped(u0, v0 + inv));
    let c11 = face_uv_to_unit(face, FaceUv::clamped(u0 + inv, v0 + inv));
    let r = angular_distance(center, c00)
        .max(angular_distance(center, c10))
        .max(angular_distance(center, c01))
        .max(angular_distance(center, c11));
    (center, r + 0.015)
}
