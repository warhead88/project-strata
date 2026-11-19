use crate::coords::EcefMeters;
use crate::lod::{CameraViewF64, LodConfig};
use crate::planet::PlanetSpec;
use crate::tile::{TileId, TileUv};

/// Bounding data for horizon and frustum culling of a globe tile.
pub(crate) struct TileBounds {
    pub(crate) center: EcefMeters,
    unit_center: EcefMeters,
    bounding_radius: f64,
    angular_radius: f64,
}

impl TileBounds {
    pub(crate) fn compute(planet: PlanetSpec, tile: TileId) -> Self {
        let center = tile.tile_uv_to_ecef(planet, TileUv::clamped(0.5, 0.5), 0.0);
        let unit_center = center.scale(1.0 / planet.radius_meters());
        let corners = [
            tile.tile_uv_to_ecef(planet, TileUv::clamped(0.0, 0.0), 0.0),
            tile.tile_uv_to_ecef(planet, TileUv::clamped(1.0, 0.0), 0.0),
            tile.tile_uv_to_ecef(planet, TileUv::clamped(1.0, 1.0), 0.0),
            tile.tile_uv_to_ecef(planet, TileUv::clamped(0.0, 1.0), 0.0),
        ];
        let mut max_dist = 0.0_f64;
        let mut max_angle = 0.0_f64;
        for corner in corners {
            max_dist = max_dist.max(center.distance_to(corner));
            let corner_unit = corner.scale(1.0 / planet.radius_meters());
            let angle = unit_center.dot(corner_unit).clamp(-1.0, 1.0).acos();
            max_angle = max_angle.max(angle);
        }
        Self {
            center,
            unit_center,
            bounding_radius: max_dist * 1.15,
            angular_radius: max_angle + 0.02,
        }
    }
}

/// Returns `true` if `bounds` is conservatively outside the planet horizon or camera frustum.
pub(crate) fn is_culled(
    planet: PlanetSpec,
    camera: &CameraViewF64,
    config: LodConfig,
    bounds: &TileBounds,
) -> bool {
    if config.enable_horizon_culling {
        let cam_r = camera.position.norm();
        if cam_r > planet.radius_meters() {
            let cam_unit = camera.position.scale(1.0 / cam_r);
            let horizon_angle = (planet.radius_meters() / cam_r).clamp(-1.0, 1.0).acos();
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

/// Closest distance in meters from `cam_pos` across a `3x3` sample grid on `tile`.
pub(crate) fn min_sample_distance(
    planet: PlanetSpec,
    cam_pos: EcefMeters,
    tile: TileId,
    center: EcefMeters,
) -> f64 {
    let mut best = cam_pos.distance_to(center);
    for iu in [0.0_f64, 0.5, 1.0] {
        for iv in [0.0_f64, 0.5, 1.0] {
            let pt = tile.tile_uv_to_ecef(planet, TileUv::clamped(iu, iv), 0.0);
            best = best.min(cam_pos.distance_to(pt));
        }
    }
    best
}
