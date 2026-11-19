use bevy::prelude::*;
use strata_globe::{CameraViewF64, LodConfig, PlanetSpec, TileId};

use crate::globe_debug::build_tile_grid_image;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct SelectionKey {
    cam_pos: [u64; 3],
    cam_fwd: [u64; 3],
    cam_right: [u64; 3],
    cam_up: [u64; 3],
    fov_y_bits: u64,
    aspect_bits: u64,
    viewport_h_bits: u64,
    planet_radius_bits: u64,
    planet_max_lod: u8,
    planet_grid_segments: u32,
    terrain_rev: u64,
    split_px_bits: u64,
    merge_px_bits: u64,
    min_lod: u8,
    roughness_bits: u64,
    horizon_culling: bool,
    frustum_culling: bool,
}

impl SelectionKey {
    pub(crate) fn new(
        cam: &CameraViewF64,
        planet: PlanetSpec,
        terrain_rev: u64,
        lod: LodConfig,
    ) -> Self {
        Self {
            cam_pos: [
                cam.position.x.to_bits(),
                cam.position.y.to_bits(),
                cam.position.z.to_bits(),
            ],
            cam_fwd: [
                cam.forward.x.to_bits(),
                cam.forward.y.to_bits(),
                cam.forward.z.to_bits(),
            ],
            cam_right: [
                cam.right.x.to_bits(),
                cam.right.y.to_bits(),
                cam.right.z.to_bits(),
            ],
            cam_up: [cam.up.x.to_bits(), cam.up.y.to_bits(), cam.up.z.to_bits()],
            fov_y_bits: cam.fov_y_rad.to_bits(),
            aspect_bits: cam.aspect.to_bits(),
            viewport_h_bits: cam.viewport_height_px.to_bits(),
            planet_radius_bits: planet.radius_meters().to_bits(),
            planet_max_lod: planet.max_lod(),
            planet_grid_segments: planet.grid_segments(),
            terrain_rev,
            split_px_bits: lod.split_threshold_px.to_bits(),
            merge_px_bits: lod.merge_threshold_px.to_bits(),
            min_lod: lod.min_lod,
            roughness_bits: lod.roughness_ratio.to_bits(),
            horizon_culling: lod.enable_horizon_culling,
            frustum_culling: lod.enable_frustum_culling,
        }
    }
}

pub(crate) struct GlobeMaterials {
    pub terrain: Handle<StandardMaterial>,
    pub water: Handle<StandardMaterial>,
    pub faces: [Handle<StandardMaterial>; 6],
}

impl GlobeMaterials {
    pub(crate) fn new(
        images: &mut Assets<Image>,
        materials: &mut Assets<StandardMaterial>,
    ) -> Self {
        let terrain = materials.add(StandardMaterial {
            base_color: Color::WHITE,
            perceptual_roughness: 0.92,
            reflectance: 0.12,
            ..default()
        });
        let water = materials.add(StandardMaterial {
            base_color: Color::WHITE,
            perceptual_roughness: 0.16,
            reflectance: 0.65,
            metallic: 0.02,
            ..default()
        });

        let grid_texture = images.add(build_tile_grid_image());
        let face_colors = [
            Color::srgb(0.28, 0.48, 0.62), // +X
            Color::srgb(0.32, 0.56, 0.50), // -X
            Color::srgb(0.58, 0.62, 0.68), // +Y (polar)
            Color::srgb(0.54, 0.58, 0.64), // -Y (polar)
            Color::srgb(0.34, 0.52, 0.42), // +Z
            Color::srgb(0.46, 0.48, 0.36), // -Z
        ];
        let faces = face_colors.map(|color| {
            materials.add(StandardMaterial {
                base_color: color,
                base_color_texture: Some(grid_texture.clone()),
                perceptual_roughness: 0.88,
                reflectance: 0.15,
                ..default()
            })
        });

        Self {
            terrain,
            water,
            faces,
        }
    }

    pub(crate) fn terrain_for(
        &self,
        tile: TileId,
        debug_overlay: bool,
    ) -> Handle<StandardMaterial> {
        if debug_overlay {
            self.faces[tile.face.index()].clone()
        } else {
            self.terrain.clone()
        }
    }
}
