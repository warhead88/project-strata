use strata_globe::{CubeFace, EcefMeters, FaceUv, ecef_to_face_uv};

use crate::hydrology::{
    LakeDef, LakeId, Navigability, RiverDef, RiverId, SEA_LEVEL_METERS, Salinity, WaterBodyKind,
    WaterSample,
};
use crate::macro_climate::{angular_distance, smoothstep};

pub(crate) const BUCKET_DIM: usize = 16;
const BUCKETS_PER_FACE: usize = BUCKET_DIM * BUCKET_DIM;
pub(crate) const TOTAL_BUCKETS: usize = 6 * BUCKETS_PER_FACE;

#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct BucketRange {
    pub lake_start: u32,
    pub lake_len: u16,
    pub seg_start: u32,
    pub seg_len: u16,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct RiverSegRef {
    pub river_idx: u16,
    pub seg_idx: u16,
}

/// Spatial `LOD 4` (`6 * 16 * 16 = 1536` buckets) hydrology index for strictly `O(1)`
/// zero-allocation lake and river queries.
#[derive(Debug, Clone)]
pub(crate) struct HydrologyIndex {
    planet_radius_m: f64,
    lakes: Vec<LakeDef>,
    rivers: Vec<RiverDef>,
    pub(crate) buckets: Vec<BucketRange>,
    pub(crate) bucket_lakes: Vec<u16>,
    pub(crate) bucket_segs: Vec<RiverSegRef>,
}

impl HydrologyIndex {
    pub(crate) fn from_parts(
        planet_radius_m: f64,
        lakes: Vec<LakeDef>,
        rivers: Vec<RiverDef>,
        buckets: Vec<BucketRange>,
        bucket_lakes: Vec<u16>,
        bucket_segs: Vec<RiverSegRef>,
    ) -> Self {
        Self {
            planet_radius_m,
            lakes,
            rivers,
            buckets,
            bucket_lakes,
            bucket_segs,
        }
    }

    pub(crate) fn lakes(&self) -> &[LakeDef] {
        &self.lakes
    }

    pub(crate) fn rivers(&self) -> &[RiverDef] {
        &self.rivers
    }

    #[inline]
    pub(crate) fn bucket_has_features(&self, face: CubeFace, lod4_x: usize, lod4_y: usize) -> bool {
        let bucket = self.buckets[bucket_index(face, lod4_x, lod4_y)];
        bucket.lake_len > 0 || bucket.seg_len > 0
    }

    /// Returns conservative `(min_m, max_m)` bounds introduced by lakes/rivers overlapping a macro cell.
    pub(crate) fn cell_bounds_contribution(
        &self,
        face: CubeFace,
        lod4_x: usize,
        lod4_y: usize,
        cell_center: EcefMeters,
        cell_half_diag_rad: f64,
    ) -> Option<(f64, f64)> {
        let bucket = self.buckets[bucket_index(face, lod4_x, lod4_y)];
        let mut min_m = f64::INFINITY;
        let mut max_m = f64::NEG_INFINITY;
        let mut touched = false;

        let l_start = bucket.lake_start as usize;
        let l_end = l_start + usize::from(bucket.lake_len);
        for &li in &self.bucket_lakes[l_start..l_end] {
            let lake = &self.lakes[usize::from(li)];
            if angular_distance(cell_center, lake.center_unit)
                <= lake.radius_rad * 1.4 + cell_half_diag_rad
            {
                min_m = min_m.min(lake.surface_elevation_m - lake.max_depth_m);
                max_m = max_m.max(lake.surface_elevation_m);
                touched = true;
            }
        }

        let s_start = bucket.seg_start as usize;
        let s_end = s_start + usize::from(bucket.seg_len);
        for &seg_ref in &self.bucket_segs[s_start..s_end] {
            let river = &self.rivers[usize::from(seg_ref.river_idx)];
            let a = river.waypoints[usize::from(seg_ref.seg_idx)];
            let b = river.waypoints[usize::from(seg_ref.seg_idx) + 1];
            let valley_rad =
                a.valley_half_width_m.max(b.valley_half_width_m) / self.planet_radius_m;
            if arc_distance_rad(cell_center, a.unit_dir, b.unit_dir)
                <= valley_rad + cell_half_diag_rad
            {
                let bed_a = a.surface_elevation_m - a.channel_depth_m;
                let bed_b = b.surface_elevation_m - b.channel_depth_m;
                min_m = min_m.min(bed_a.min(bed_b));
                max_m = max_m.max(a.surface_elevation_m.max(b.surface_elevation_m));
                touched = true;
            }
        }

        if touched { Some((min_m, max_m)) } else { None }
    }

    /// Evaluates river/lake carving and water sample at `unit_dir` in strictly `O(1)` time.
    #[must_use]
    pub(crate) fn carve_and_sample(
        &self,
        unit_dir: EcefMeters,
        base_elev_m: f64,
    ) -> (f64, WaterSample) {
        if self.lakes.is_empty() && self.rivers.is_empty() {
            return dry_or_ocean(base_elev_m);
        }
        let Ok((face, face_uv)) = ecef_to_face_uv(unit_dir) else {
            return dry_or_ocean(base_elev_m);
        };
        self.carve_and_sample_uv(face, face_uv, unit_dir, base_elev_m)
    }

    /// Evaluates river/lake carving and water sample using precomputed `(face, face_uv)`.
    #[inline]
    #[must_use]
    pub(crate) fn carve_and_sample_uv(
        &self,
        face: CubeFace,
        face_uv: FaceUv,
        unit_dir: EcefMeters,
        base_elev_m: f64,
    ) -> (f64, WaterSample) {
        if self.lakes.is_empty() && self.rivers.is_empty() {
            return dry_or_ocean(base_elev_m);
        }
        let (bx, by) = face_uv_to_lod4(face_uv);
        let bucket = self.buckets[bucket_index(face, bx, by)];
        if bucket.lake_len == 0 && bucket.seg_len == 0 {
            return dry_or_ocean(base_elev_m);
        }

        let (elev, lake_hit) = if bucket.lake_len > 0 {
            self.apply_lakes(bucket, unit_dir, base_elev_m)
        } else {
            (base_elev_m, None)
        };
        if let Some(sample) = lake_hit {
            return (elev, sample);
        }

        let (elev, river_hit) = if bucket.seg_len > 0 {
            self.apply_rivers(bucket, unit_dir, elev)
        } else {
            (elev, None)
        };
        if let Some(sample) = river_hit {
            return (elev, sample);
        }

        dry_or_ocean(elev)
    }

    /// Fast elevation-only hydrology carving using precomputed `(face, face_uv)`.
    #[inline]
    #[must_use]
    pub(crate) fn carve_elevation_uv(
        &self,
        face: CubeFace,
        face_uv: FaceUv,
        unit_dir: EcefMeters,
        base_elev_m: f64,
    ) -> f64 {
        if self.lakes.is_empty() && self.rivers.is_empty() {
            return base_elev_m;
        }
        let (bx, by) = face_uv_to_lod4(face_uv);
        let bucket = self.buckets[bucket_index(face, bx, by)];
        if bucket.lake_len == 0 && bucket.seg_len == 0 {
            return base_elev_m;
        }
        let (elev, lake_hit) = if bucket.lake_len > 0 {
            self.apply_lakes(bucket, unit_dir, base_elev_m)
        } else {
            (base_elev_m, None)
        };
        if lake_hit.is_some() {
            return elev;
        }
        if bucket.seg_len > 0 {
            let (elev, _) = self.apply_rivers(bucket, unit_dir, elev);
            elev
        } else {
            elev
        }
    }

    fn apply_lakes(
        &self,
        bucket: BucketRange,
        unit_dir: EcefMeters,
        base_elev_m: f64,
    ) -> (f64, Option<WaterSample>) {
        let mut elev = base_elev_m;
        let mut active: Option<(LakeId, f64, f64, Salinity)> = None;

        let l_start = bucket.lake_start as usize;
        let l_end = l_start + usize::from(bucket.lake_len);
        for &li in &self.bucket_lakes[l_start..l_end] {
            let lake = &self.lakes[usize::from(li)];
            let ang = angular_distance(unit_dir, lake.center_unit);
            let collar_rad = lake.radius_rad * 1.38;
            if ang < collar_rad {
                if ang < lake.radius_rad {
                    let t = (ang / lake.radius_rad).clamp(0.0, 1.0);
                    let depth = lake.max_depth_m * (1.0 - t * t).powf(0.85);
                    let bed = lake.surface_elevation_m - depth;
                    if active.is_none_or(|(_, _, prev_d, _)| depth > prev_d) {
                        active = Some((lake.id, lake.surface_elevation_m, depth, lake.salinity));
                        elev = bed;
                    }
                } else if active.is_none() {
                    let blend = smoothstep(collar_rad, lake.radius_rad, ang) * 0.9;
                    elev += (lake.surface_elevation_m - elev) * blend;
                }
            }
        }

        let hit = active.map(|(id, surf_m, depth_m, salinity)| WaterSample {
            kind: WaterBodyKind::Lake(id),
            surface_elevation_m: surf_m,
            depth_m,
            discharge_m3s: 0.0,
            navigability: Navigability::Unnavigable,
            is_freshwater: salinity == Salinity::Freshwater,
        });
        (elev, hit)
    }

    fn apply_rivers(
        &self,
        bucket: BucketRange,
        unit_dir: EcefMeters,
        base_elev_m: f64,
    ) -> (f64, Option<WaterSample>) {
        let mut elev = base_elev_m;
        let mut best: Option<(RiverId, Navigability, f64, f64, f64, f64)> = None;

        let s_start = bucket.seg_start as usize;
        let s_end = s_start + usize::from(bucket.seg_len);
        for &seg_ref in &self.bucket_segs[s_start..s_end] {
            let river = &self.rivers[usize::from(seg_ref.river_idx)];
            let a = river.waypoints[usize::from(seg_ref.seg_idx)];
            let b = river.waypoints[usize::from(seg_ref.seg_idx) + 1];
            let (closest, s) = closest_on_unit_segment(unit_dir, a.unit_dir, b.unit_dir);
            let dist_m = angular_distance(unit_dir, closest) * self.planet_radius_m;

            let valley_hw =
                a.valley_half_width_m + (b.valley_half_width_m - a.valley_half_width_m) * s;
            if dist_m >= valley_hw {
                continue;
            }
            let half_w = a.half_width_m + (b.half_width_m - a.half_width_m) * s;
            let surf = a.surface_elevation_m + (b.surface_elevation_m - a.surface_elevation_m) * s;
            let c_depth = a.channel_depth_m + (b.channel_depth_m - a.channel_depth_m) * s;
            let disch = a.discharge_m3s + (b.discharge_m3s - a.discharge_m3s) * s;
            let ratio = dist_m / half_w.max(1.0);

            if best.is_none_or(|(_, _, _, _, _, prev_r)| ratio < prev_r) {
                let valley_blend = smoothstep(valley_hw, half_w, dist_m) * 0.85;
                let carved_valley = elev + (surf - elev) * valley_blend;
                let depth_m = if dist_m < half_w {
                    c_depth * (1.0 - ratio * ratio)
                } else {
                    0.0
                };
                best = Some((
                    river.id,
                    river.navigability,
                    surf,
                    depth_m,
                    disch,
                    if dist_m < half_w {
                        ratio
                    } else {
                        valley_hw / half_w.max(1.0) + ratio
                    },
                ));
                elev = if depth_m > 0.0 {
                    surf - depth_m
                } else {
                    carved_valley
                };
            }
        }

        let hit = best.and_then(|(id, nav, surf, depth_m, disch, _)| {
            (depth_m > 0.0).then_some(WaterSample {
                kind: WaterBodyKind::River(id),
                surface_elevation_m: surf,
                depth_m,
                discharge_m3s: disch,
                navigability: nav,
                is_freshwater: true,
            })
        });
        (elev, hit)
    }
}

fn dry_or_ocean(elev_m: f64) -> (f64, WaterSample) {
    if elev_m < SEA_LEVEL_METERS {
        (
            elev_m,
            WaterSample {
                kind: WaterBodyKind::Ocean,
                surface_elevation_m: SEA_LEVEL_METERS,
                depth_m: -elev_m,
                discharge_m3s: 0.0,
                navigability: Navigability::Unnavigable,
                is_freshwater: false,
            },
        )
    } else {
        (
            elev_m,
            WaterSample {
                kind: WaterBodyKind::Dry,
                surface_elevation_m: elev_m,
                depth_m: 0.0,
                discharge_m3s: 0.0,
                navigability: Navigability::Unnavigable,
                is_freshwater: true,
            },
        )
    }
}

#[inline]
fn bucket_index(face: CubeFace, x: usize, y: usize) -> usize {
    face.index() * BUCKETS_PER_FACE + y * BUCKET_DIM + x
}

#[inline]
#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "FaceUv is clamped to [0, 1] so indices are strictly in 0..16"
)]
fn face_uv_to_lod4(uv: FaceUv) -> (usize, usize) {
    let x = ((uv.u.clamp(0.0, 1.0) * 16.0).floor() as usize).min(BUCKET_DIM - 1);
    let y = ((uv.v.clamp(0.0, 1.0) * 16.0).floor() as usize).min(BUCKET_DIM - 1);
    (x, y)
}

pub(crate) fn closest_on_unit_segment(
    p: EcefMeters,
    a: EcefMeters,
    b: EcefMeters,
) -> (EcefMeters, f64) {
    let ab = b - a;
    let ab2 = ab.dot(ab);
    if ab2 <= 1e-12 {
        return (a, 0.0);
    }
    let s = ((p - a).dot(ab) / ab2).clamp(0.0, 1.0);
    let raw = a + ab.scale(s);
    let len = raw.norm();
    let unit = if len > 0.0 { raw.scale(1.0 / len) } else { a };
    (unit, s)
}

pub(crate) fn arc_distance_rad(p: EcefMeters, a: EcefMeters, b: EcefMeters) -> f64 {
    let (closest, _) = closest_on_unit_segment(p, a, b);
    angular_distance(p, closest)
}
