use std::collections::{HashMap, HashSet};
use std::hash::{BuildHasherDefault, Hasher};

use crate::coords::EcefMeters;
use crate::cube::face_uv_to_unit;
use crate::elevation::{ElevationRange, ElevationSampler};
use crate::error::GlobeError;
use crate::lod::{CameraViewF64, LodConfig};
use crate::planet::PlanetSpec;
use crate::tile::{TileId, TileUv};

#[derive(Default)]
pub(crate) struct TileHasher(u64);

impl Hasher for TileHasher {
    #[inline]
    fn finish(&self) -> u64 {
        self.0
    }

    #[inline]
    fn write(&mut self, bytes: &[u8]) {
        for &b in bytes {
            self.0 = (self.0.rotate_left(5) ^ u64::from(b)).wrapping_mul(0x517c_c1b7_2722_0a95);
        }
    }

    #[inline]
    fn write_u8(&mut self, i: u8) {
        self.0 = (self.0.rotate_left(5) ^ u64::from(i)).wrapping_mul(0x517c_c1b7_2722_0a95);
    }

    #[inline]
    fn write_u32(&mut self, i: u32) {
        self.0 = (self.0.rotate_left(5) ^ u64::from(i)).wrapping_mul(0x517c_c1b7_2722_0a95);
    }

    #[inline]
    fn write_u64(&mut self, i: u64) {
        self.0 = (self.0.rotate_left(5) ^ i).wrapping_mul(0x517c_c1b7_2722_0a95);
    }
}

pub(crate) type FastTileSet = HashSet<TileId, BuildHasherDefault<TileHasher>>;
pub(crate) type FastTileMap<V> = HashMap<TileId, V, BuildHasherDefault<TileHasher>>;

/// Frame-constant camera and horizon culling precomputations.
pub(crate) struct CullingFrame {
    cam_pos: EcefMeters,
    cam_unit: EcefMeters,
    cam_r: f64,
    r_occ: f64,
    cam_horizon: f64,
    can_horizon_cull: bool,
    cos_y: f64,
    sin_y: f64,
    cos_x: f64,
    sin_x: f64,
}

impl CullingFrame {
    pub(crate) fn new(
        planet: PlanetSpec,
        camera: &CameraViewF64,
        global_range: ElevationRange,
    ) -> Self {
        let r_occ = global_range.min_radius_meters(planet);
        let cam_r = camera.position.norm();
        let can_horizon_cull = cam_r > r_occ;
        let cam_unit = if cam_r > 0.0 {
            camera.position.scale(1.0 / cam_r)
        } else {
            EcefMeters::from_raw(0.0, 0.0, 1.0)
        };
        let cam_horizon = if can_horizon_cull {
            (r_occ / cam_r).clamp(-1.0, 1.0).acos()
        } else {
            0.0
        };

        let half_tan_y = (camera.fov_y_rad * 0.5).tan();
        let cos_y = 1.0 / (1.0 + half_tan_y * half_tan_y).sqrt();
        let sin_y = half_tan_y * cos_y;
        let half_tan_x = half_tan_y * camera.aspect;
        let cos_x = 1.0 / (1.0 + half_tan_x * half_tan_x).sqrt();
        let sin_x = half_tan_x * cos_x;

        Self {
            cam_pos: camera.position,
            cam_unit,
            cam_r,
            r_occ,
            cam_horizon,
            can_horizon_cull,
            cos_y,
            sin_y,
            cos_x,
            sin_x,
        }
    }
}

/// Combined culling test and conservative closest-distance evaluation sharing tile sample directions.
#[inline]
pub(crate) fn eval_tile_visibility_and_distance(
    planet: PlanetSpec,
    frame: &CullingFrame,
    camera: &CameraViewF64,
    config: LodConfig,
    tile: TileId,
    tile_range: ElevationRange,
) -> Option<f64> {
    let range = tile_range.clamp_planet_radius(planet);
    let r = planet.radius_meters();
    let min_r = 1.0_f64.min(r * 0.5);
    let mid_elev = 0.5 * (range.min_meters + range.max_meters);

    let unit_center = face_uv_to_unit(
        tile.face,
        tile.tile_uv_to_face_uv(TileUv::clamped(0.5, 0.5)),
    );
    let center = unit_center.scale((r + mid_elev).max(min_r));

    let corner_units = [
        face_uv_to_unit(
            tile.face,
            tile.tile_uv_to_face_uv(TileUv::clamped(0.0, 0.0)),
        ),
        face_uv_to_unit(
            tile.face,
            tile.tile_uv_to_face_uv(TileUv::clamped(1.0, 0.0)),
        ),
        face_uv_to_unit(
            tile.face,
            tile.tile_uv_to_face_uv(TileUv::clamped(1.0, 1.0)),
        ),
        face_uv_to_unit(
            tile.face,
            tile.tile_uv_to_face_uv(TileUv::clamped(0.0, 1.0)),
        ),
    ];

    let r_tile_min = (r + range.min_meters).max(min_r);
    let r_tile_max = (r + range.max_meters).max(min_r);

    let mut max_dist = center
        .distance_to(unit_center.scale(r_tile_min))
        .max(center.distance_to(unit_center.scale(r_tile_max)));
    let mut min_cos = 1.0_f64;

    for corner_unit in corner_units {
        max_dist = max_dist
            .max(center.distance_to(corner_unit.scale(r_tile_min)))
            .max(center.distance_to(corner_unit.scale(r_tile_max)));
        min_cos = min_cos.min(unit_center.dot(corner_unit));
    }

    let bounding_radius = max_dist * 1.15;
    let angular_radius = min_cos.clamp(-1.0, 1.0).acos() + 0.02;

    if config.enable_horizon_culling && frame.can_horizon_cull && frame.cam_r > frame.r_occ {
        let r_max = r_tile_max.max(frame.r_occ);
        let peak_horizon = (frame.r_occ / r_max).clamp(-1.0, 1.0).acos();
        let limit_angle = frame.cam_horizon + peak_horizon + angular_radius;
        if limit_angle < std::f64::consts::PI {
            let center_dot = frame.cam_unit.dot(unit_center);
            if center_dot < limit_angle.cos() {
                return None;
            }
        }
    }

    if config.enable_frustum_culling {
        let rel = center - frame.cam_pos;
        let z = rel.dot(camera.forward);
        if z + bounding_radius < 0.0 {
            return None;
        }
        let y = rel.dot(camera.up);
        if y.abs() * frame.cos_y - z * frame.sin_y > bounding_radius {
            return None;
        }
        let x = rel.dot(camera.right);
        if x.abs() * frame.cos_x - z * frame.sin_x > bounding_radius {
            return None;
        }
    }

    let edge_units = [
        face_uv_to_unit(
            tile.face,
            tile.tile_uv_to_face_uv(TileUv::clamped(0.5, 0.0)),
        ),
        face_uv_to_unit(
            tile.face,
            tile.tile_uv_to_face_uv(TileUv::clamped(0.0, 0.5)),
        ),
        face_uv_to_unit(
            tile.face,
            tile.tile_uv_to_face_uv(TileUv::clamped(1.0, 0.5)),
        ),
        face_uv_to_unit(
            tile.face,
            tile.tile_uv_to_face_uv(TileUv::clamped(0.5, 1.0)),
        ),
    ];

    let mut best = closest_point_dist(frame.cam_pos, unit_center, r_tile_min, r_tile_max);
    for u in corner_units {
        best = best.min(closest_point_dist(frame.cam_pos, u, r_tile_min, r_tile_max));
    }
    for u in edge_units {
        best = best.min(closest_point_dist(frame.cam_pos, u, r_tile_min, r_tile_max));
    }
    Some(best)
}

/// Conservative closest distance in meters from `cam_pos` across a `3x3` sample grid
/// on `tile`, bounded over `[r_min, r_max]` from `tile_range` with zero `sample_elevation` calls.
#[inline]
pub(crate) fn min_sample_distance_from_range(
    planet: PlanetSpec,
    cam_pos: EcefMeters,
    tile: TileId,
    tile_range: ElevationRange,
) -> f64 {
    let clamped = tile_range.clamp_planet_radius(planet);
    let r_base = planet.radius_meters();
    let r_min = r_base + clamped.min_meters;
    let r_max = r_base + clamped.max_meters;
    let mut best = f64::INFINITY;
    for iu in [0.0_f64, 0.5, 1.0] {
        for iv in [0.0_f64, 0.5, 1.0] {
            let face_uv = tile.tile_uv_to_face_uv(TileUv::clamped(iu, iv));
            let unit = face_uv_to_unit(tile.face, face_uv);
            best = best.min(closest_point_dist(cam_pos, unit, r_min, r_max));
        }
    }
    best
}

#[inline]
fn closest_point_dist(cam_pos: EcefMeters, unit: EcefMeters, r_min: f64, r_max: f64) -> f64 {
    let r_closest = cam_pos.dot(unit).clamp(r_min, r_max);
    cam_pos.distance_to(unit.scale(r_closest))
}

pub(crate) fn validate_selection_inputs(
    camera: &CameraViewF64,
    config: LodConfig,
    sampler: &dyn ElevationSampler,
) -> Result<(), GlobeError> {
    if !camera.position.is_finite()
        || !camera.forward.is_finite()
        || !camera.right.is_finite()
        || !camera.up.is_finite()
        || !config.split_threshold_px.is_finite()
        || !config.merge_threshold_px.is_finite()
        || !config.roughness_ratio.is_finite()
    {
        return Err(GlobeError::NonFiniteCoordinate);
    }
    if !camera.fov_y_rad.is_finite()
        || camera.fov_y_rad <= 0.0
        || camera.fov_y_rad >= std::f64::consts::PI
        || !camera.aspect.is_finite()
        || camera.aspect <= 0.0
        || !camera.viewport_height_px.is_finite()
        || camera.viewport_height_px <= 0.0
    {
        return Err(GlobeError::InvalidCameraProjection);
    }
    let range = sampler.global_range();
    if !range.min_meters.is_finite()
        || !range.max_meters.is_finite()
        || range.min_meters > range.max_meters
    {
        return Err(GlobeError::InvalidElevationRange {
            min_meters: range.min_meters,
            max_meters: range.max_meters,
        });
    }
    Ok(())
}

#[inline]
pub(crate) fn tile_geometric_error_meters(
    planet: PlanetSpec,
    tile: TileId,
    config: LodConfig,
    tile_range: ElevationRange,
) -> f64 {
    let base_err = planet.lod_geometric_error_meters(tile.lod, config.roughness_ratio);
    if tile_range.is_flat() {
        return base_err;
    }
    let cell_arc = planet.cell_arc_length_meters(tile.lod);
    let relief_err =
        (tile_range.span_meters() / f64::from(planet.grid_segments())).min(cell_arc * 0.5);
    base_err.max(relief_err)
}
