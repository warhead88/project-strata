use std::f64::consts::{FRAC_PI_2, FRAC_PI_4};

use bevy::{
    math::Mat4,
    render::{camera::ExtractedCamera, view::ExtractedView},
};
use bytemuck::{Pod, Zeroable};
use strata_globe::{CubeFace, EcefMeters, FaceUv, face_uv_to_unit};

use super::{
    ExtractedTerrain,
    source::{SourceSettings, meters_f32, vec4_array},
};

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub(super) struct FaceAnchor {
    pub(super) relative: [f32; 4],
    unit_length: [f32; 4],
    pub(super) uv: [f32; 4],
    tangent: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub(super) struct GpuView {
    clip_from_eye: [[f32; 4]; 4],
    pub(super) anchors: [FaceAnchor; 6],
    pub(super) planet: [f32; 4],
    eye: [f32; 4],
    pub(super) settings: [u32; 4],
    source: [u32; 4],
    detail: [u32; 4],
    material_origin: [f32; 4],
}

impl ExtractedTerrain {
    pub(super) fn view(
        &self,
        camera: &ExtractedCamera,
        view: &ExtractedView,
        source: SourceSettings,
    ) -> GpuView {
        let rotation = Mat4::from_quat(view.world_from_view.rotation().inverse());
        let projection_scale = camera
            .physical_viewport_size
            .map_or(720.0, |size| meters_f32(f64::from(size.y)))
            * view.clip_from_view.y_axis.y
            * 0.5;
        self.frame(view.clip_from_view * rotation, projection_scale, source)
    }

    pub(super) fn frame(
        &self,
        clip_from_eye: Mat4,
        projection_scale: f32,
        source: SourceSettings,
    ) -> GpuView {
        let planet = self.camera.planet;
        let radius = planet.radius_meters();
        let (eye, _, _) = self.camera.compute_pose_f64();
        let radial = eye.norm();
        let dir = eye.scale(1.0 / radial);
        let range = self
            .terrain
            .sampler()
            .global_range()
            .clamp_planet_radius(planet);
        let clearance = (radial - radius - self.camera.eye_ground_elevation_m.max(0.0)).max(20.0);
        GpuView {
            clip_from_eye: clip_from_eye.to_cols_array_2d(),
            anchors: CubeFace::ALL.map(|face| face_anchor(face, eye, dir, radius)),
            planet: [
                meters_f32(radius),
                // Account for decimation/curvature of the coarsest rendered shell.
                meters_f32(range.min_radius_meters(planet) - planet.tile_surface_sagitta_meters(4)),
                meters_f32(radial),
                projection_scale,
            ],
            eye: vec4_array(dir, clearance),
            settings: [
                u32::from(planet.max_lod()),
                u32::from(self.debug),
                planet.grid_segments(),
                source.pixel_centered,
            ],
            source: [
                source.macro_resolution,
                source.second_octave_offset,
                source.bucket_dim,
                source.frequencies[0],
            ],
            detail: [
                source.frequencies[1],
                source.hydrology_radius_m.to_bits(),
                0,
                0,
            ],
            material_origin: [
                eye.x.rem_euclid(256.0),
                eye.y.rem_euclid(256.0),
                eye.z.rem_euclid(256.0),
                0.0,
            ]
            .map(meters_f32),
        }
    }
}

pub(super) fn face_anchor(
    face: CubeFace,
    eye: EcefMeters,
    dir: EcefMeters,
    radius: f64,
) -> FaceAnchor {
    let basis = face.basis();
    let n = dir.dot(basis.normal);
    let coord = |axis| (dir.dot(axis).atan2(n) / FRAC_PI_2 + 0.5).clamp(0.0, 1.0);
    // Quantize the chart anchor first; the f64 ECEF origin then represents exactly
    // the same chart coordinate used in the GPU subtraction.
    let u = meters_f32(coord(basis.right));
    let v = meters_f32(coord(basis.up));
    let uv = FaceUv {
        u: f64::from(u),
        v: f64::from(v),
    };
    let unit = face_uv_to_unit(face, uv);
    let tangent = |coord: f64| {
        let angle = 2.0 * coord - 1.0;
        if angle.abs() >= 1.0 {
            angle
        } else {
            (angle * FRAC_PI_4).tan()
        }
    };
    FaceAnchor {
        relative: vec4_array(unit.scale(radius) - eye, 0.0),
        unit_length: vec4_array(unit, unit.dot(basis.normal).recip()),
        uv: [u, v, 0.0, 0.0],
        tangent: [
            meters_f32(tangent(uv.u)),
            meters_f32(tangent(uv.v)),
            0.0,
            0.0,
        ],
    }
}
