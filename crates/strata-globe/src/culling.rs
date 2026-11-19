use crate::coords::EcefMeters;
use crate::cube::face_uv_to_unit;
use crate::elevation::{ElevationRange, ElevationSampler};
use crate::error::GlobeError;
use crate::lod::{CameraViewF64, LodConfig};
use crate::planet::PlanetSpec;
use crate::tile::{TileId, TileUv};

/// Bounding data for horizon and frustum culling of a displaced globe tile.
pub(crate) struct TileBounds {
    pub(crate) center: EcefMeters,
    unit_center: EcefMeters,
    bounding_radius: f64,
    angular_radius: f64,
    max_radius_meters: f64,
}

impl TileBounds {
    pub(crate) fn compute(planet: PlanetSpec, tile: TileId, tile_range: ElevationRange) -> Self {
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
        let mut max_angle = 0.0_f64;

        for corner_unit in corner_units {
            max_dist = max_dist
                .max(center.distance_to(corner_unit.scale(r_tile_min)))
                .max(center.distance_to(corner_unit.scale(r_tile_max)));
            let angle = unit_center.dot(corner_unit).clamp(-1.0, 1.0).acos();
            max_angle = max_angle.max(angle);
        }

        Self {
            center,
            unit_center,
            bounding_radius: max_dist * 1.15,
            angular_radius: max_angle + 0.02,
            max_radius_meters: r_tile_max,
        }
    }
}

/// Returns `true` if `bounds` is conservatively outside the planet horizon or camera frustum.
pub(crate) fn is_culled(
    planet: PlanetSpec,
    camera: &CameraViewF64,
    config: LodConfig,
    bounds: &TileBounds,
    global_range: ElevationRange,
) -> bool {
    if config.enable_horizon_culling {
        let r_occ = global_range.min_radius_meters(planet);
        let r_max = bounds.max_radius_meters.max(r_occ);
        let cam_r = camera.position.norm();
        if cam_r > r_occ {
            let cam_unit = camera.position.scale(1.0 / cam_r);
            let cam_horizon = (r_occ / cam_r).clamp(-1.0, 1.0).acos();
            let peak_horizon = (r_occ / r_max).clamp(-1.0, 1.0).acos();
            let horizon_angle = cam_horizon + peak_horizon;
            let center_angle = cam_unit.dot(bounds.unit_center).clamp(-1.0, 1.0).acos();
            if center_angle > horizon_angle + bounds.angular_radius {
                return true;
            }
        }
    }

    if config.enable_frustum_culling {
        let rel = bounds.center - camera.position;
        let z = rel.dot(camera.forward);
        if z + bounds.bounding_radius < 0.0 {
            return true;
        }
        let x = rel.dot(camera.right);
        let y = rel.dot(camera.up);
        let half_tan_y = (camera.fov_y_rad * 0.5).tan();
        let cos_y = 1.0 / (1.0 + half_tan_y * half_tan_y).sqrt();
        let sin_y = half_tan_y * cos_y;
        if y.abs() * cos_y - z * sin_y > bounds.bounding_radius {
            return true;
        }
        let half_tan_x = half_tan_y * camera.aspect;
        let cos_x = 1.0 / (1.0 + half_tan_x * half_tan_x).sqrt();
        let sin_x = half_tan_x * cos_x;
        if x.abs() * cos_x - z * sin_x > bounds.bounding_radius {
            return true;
        }
    }

    false
}

/// Closest distance in meters from `cam_pos` across a `3x3` displaced sample grid on `tile`.
pub(crate) fn min_sample_distance(
    planet: PlanetSpec,
    cam_pos: EcefMeters,
    tile: TileId,
    sampler: &dyn ElevationSampler,
) -> f64 {
    let is_flat = sampler.global_range().is_flat();
    let r_base = planet.radius_meters();
    let min_r = 1.0_f64.min(r_base * 0.5);
    let mut best = f64::INFINITY;
    for iu in [0.0_f64, 0.5, 1.0] {
        for iv in [0.0_f64, 0.5, 1.0] {
            let face_uv = tile.tile_uv_to_face_uv(TileUv::clamped(iu, iv));
            let unit = face_uv_to_unit(tile.face, face_uv);
            let elev = if is_flat {
                0.0
            } else {
                sampler.sample_elevation(unit)
            };
            let r = if elev.is_finite() {
                (r_base + elev).max(min_r)
            } else {
                r_base
            };
            best = best.min(cam_pos.distance_to(unit.scale(r)));
        }
    }
    best
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
