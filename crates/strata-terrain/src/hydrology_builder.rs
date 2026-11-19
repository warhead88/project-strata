use strata_globe::{
    CubeFace, EARTH_MEAN_RADIUS_METERS, EcefMeters, FaceUv, PlanetSpec, face_uv_to_unit,
};

use crate::earth_geography::{TerrainSource, lat_lon_to_unit};
use crate::error::TerrainError;
use crate::hydrology::{LakeDef, LakeId, RiverDef, RiverId, RiverWaypoint};
use crate::hydrology_data::{EARTH_LAKES, EARTH_RIVERS};
use crate::hydrology_index::{
    BUCKET_DIM, BucketRange, HydrologyIndex, RiverSegRef, TOTAL_BUCKETS, arc_distance_rad,
};
use crate::macro_climate::angular_distance;

pub(crate) fn build_hydrology_index(
    planet: PlanetSpec,
    source: &TerrainSource,
) -> Result<HydrologyIndex, TerrainError> {
    let radius_m = planet.radius_meters();
    let scale = (radius_m / EARTH_MEAN_RADIUS_METERS).clamp(0.02, 4.0);
    let (lakes, rivers) = match source {
        TerrainSource::Earth { .. } => build_earth_hydrology(scale)?,
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
