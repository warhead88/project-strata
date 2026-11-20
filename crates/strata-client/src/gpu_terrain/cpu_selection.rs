use std::f64::consts::FRAC_PI_2;

use bevy::{math::Mat4, prelude::*};
use strata_globe::{
    EcefMeters, ElevationSampler, PlanetSpec, TileEdge, TileId, TileUv, face_uv_to_unit,
};

use super::{ExtractedTerrain, view::GpuView};

#[derive(Default)]
pub(super) struct CpuSelection {
    pub(super) instances: Vec<[u32; 4]>,
    pending: Vec<TileId>,
}

struct Camera {
    planet: PlanetSpec,
    position: EcefMeters,
    direction: EcefMeters,
    eye_radius_m: f64,
    occluder_m: f64,
    projection_scale: f64,
    clearance_m: f64,
    clip_from_eye: Mat4,
    frustum_x: f32,
    frustum_y: f32,
}

impl CpuSelection {
    pub(super) fn update(&mut self, input: &ExtractedTerrain, view: &GpuView) {
        let camera = Camera::new(input, view);
        self.update_for_camera(&camera, input.terrain.sampler());
    }

    fn update_for_camera(&mut self, camera: &Camera, elevation: &dyn ElevationSampler) {
        self.instances.clear();
        self.pending.clear();
        self.pending.extend(TileId::roots());
        while let Some(tile) = self.pending.pop() {
            let center = tile_center(tile);
            let angle = tile_angle(tile);
            let range = elevation.tile_range(camera.planet, tile);
            if !camera.visible(center, angle, range.min_meters, range.max_meters) {
                continue;
            }
            if camera.must_split(tile, center, angle)
                && let Some(children) = tile.children()
            {
                self.pending.extend(children);
                continue;
            }
            let mut edges = 0;
            for (bit, edge) in [
                TileEdge::Left,
                TileEdge::Right,
                TileEdge::Bottom,
                TileEdge::Top,
            ]
            .into_iter()
            .enumerate()
            {
                if let Some(neighbor) = tile.neighbor(edge).tile.parent()
                    && !camera.must_split(neighbor, tile_center(neighbor), tile_angle(neighbor))
                {
                    edges |= 1 << bit;
                }
            }
            self.instances.push([
                tile.x,
                tile.y,
                u32::from(tile.lod),
                u32::from(tile.face as u8) | (edges << 3),
            ]);
        }
    }
}

impl Camera {
    fn new(input: &ExtractedTerrain, view: &GpuView) -> Self {
        let (position, _, _) = input.camera.compute_pose_f64();
        let radius_m = position.norm();
        let clip_from_eye = Mat4::from_cols_array_2d(&view.clip_from_eye);
        let rows = clip_from_eye.transpose();
        Self {
            planet: input.camera.planet,
            position,
            direction: position.scale(radius_m.recip()),
            eye_radius_m: f64::from(view.planet[2]),
            occluder_m: f64::from(view.planet[1]),
            projection_scale: f64::from(view.planet[3]),
            clearance_m: f64::from(view.eye[3]),
            clip_from_eye,
            frustum_x: (rows.x_axis + rows.w_axis).truncate().length(),
            frustum_y: (rows.y_axis + rows.w_axis).truncate().length(),
        }
    }

    fn must_split(&self, tile: TileId, center: EcefMeters, angle: f64) -> bool {
        if tile.lod >= self.planet.max_lod() {
            return false;
        }
        if tile.lod < 4 {
            return true;
        }
        let radius = self.planet.radius_meters();
        let grid = f64::from(self.planet.grid_segments());
        let cell_m = radius * angle / grid;
        let sagitta_m = angle * angle * radius / (4.0 * grid * grid);
        center.distance_to(self.direction) < 3.0 * angle
            && sagitta_m.max(cell_m * 0.30) * self.projection_scale > 3.5 * self.clearance_m
    }

    #[expect(
        clippy::cast_possible_truncation,
        reason = "camera-relative positions narrow to exactly the GPU projection format"
    )]
    fn visible(&self, center: EcefMeters, angle: f64, low: f64, high: f64) -> bool {
        let radius = self.planet.radius_meters();
        let cap = angle * std::f64::consts::SQRT_2;
        let high = high.max(0.0);
        let top = radius + high;
        if self.eye_radius_m > self.occluder_m {
            let horizon = (self.occluder_m / self.eye_radius_m).clamp(0.0, 1.0).acos()
                + (self.occluder_m / top).clamp(0.0, 1.0).acos()
                + cap;
            if horizon < std::f64::consts::PI && center.dot(self.direction) < horizon.cos() {
                return false;
            }
        }
        let mid = f64::midpoint(low, high);
        let relative = center.scale(radius + mid) - self.position;
        let size = (cap * top + (high - mid) + 5.0) as f32;
        let clip = self.clip_from_eye
            * Vec4::new(relative.x as f32, relative.y as f32, relative.z as f32, 1.0);
        clip.x.abs() - clip.w <= size * self.frustum_x
            && clip.y.abs() - clip.w <= size * self.frustum_y
            && clip.w >= -size
    }
}

fn tile_center(tile: TileId) -> EcefMeters {
    face_uv_to_unit(
        tile.face,
        tile.tile_uv_to_face_uv(TileUv::clamped(0.5, 0.5)),
    )
}

fn tile_angle(tile: TileId) -> f64 {
    FRAC_PI_2 / f64::from(1_u32 << tile.lod)
}
