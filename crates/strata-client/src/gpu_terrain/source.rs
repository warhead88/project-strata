use bevy::render::{
    render_resource::{
        Buffer, BufferInitDescriptor, BufferUsages, Extent3d, TextureDataOrder, TextureDescriptor,
        TextureDimension, TextureFormat, TextureUsages, TextureView, TextureViewDescriptor,
    },
    renderer::{RenderDevice, RenderQueue},
};
use bytemuck::{Pod, Zeroable, cast_slice};
use strata_globe::{CubeFace, EcefMeters, TileId};
use strata_terrain::{SurfaceData, SurfaceRaster};

use super::earth_textures::EarthTextures;

#[derive(Clone, Copy, Pod, Zeroable)]
#[repr(C)]
pub(super) struct SourceSettings {
    pub(super) macro_resolution: u32,
    pub(super) second_octave_offset: u32,
    pub(super) pixel_centered: u32,
    pub(super) frequencies: [u32; 2],
}

#[derive(Clone)]
pub(super) struct SourceGpu {
    pub(super) settings: SourceSettings,
    pub(super) bounds: Buffer,
    pub(super) earth: EarthTextures,
    pub(super) gradients: Buffer,
    pub(super) dem: TextureView,
    pub(super) climate: TextureView,
}

impl SourceGpu {
    pub(super) fn upload(
        device: &RenderDevice,
        queue: &RenderQueue,
        data: &SurfaceData,
    ) -> Option<Self> {
        let (width, height, raster) = data.raster();
        let (width, height, heights, earth_data) = match raster {
            SurfaceRaster::Earth(values) => (1, 1, &[0.0][..], Some((values, data.earth_water()?))),
            SurfaceRaster::Dem(values) => (
                u32::try_from(width).ok()?,
                u32::try_from(height).ok()?,
                values,
                None,
            ),
        };
        if width.max(height) > device.limits().max_texture_dimension_2d {
            return None;
        }
        let earth = EarthTextures::upload(device, queue, earth_data)?;
        let dem = upload_texture(
            device,
            queue,
            "Terrain DEM",
            [width, height, 1],
            TextureFormat::R32Float,
            heights,
        );
        let (resolution, macro_grid) = data.macro_grid();
        let fields: Vec<_> = macro_grid
            .iter()
            .map(|cell| {
                [
                    cell.macro_elevation_m,
                    cell.roughness,
                    cell.mean_temp_c,
                    cell.annual_precip_mm,
                ]
            })
            .collect();
        let climate = upload_texture(
            device,
            queue,
            "Terrain macro fields",
            [resolution + 1, resolution + 1, 6],
            TextureFormat::Rgba32Float,
            &fields,
        );
        let octaves = data.detail_octaves();
        let gradients = super::gradients::generate(device, queue, octaves);
        let bounds = upload_bounds(device, data, resolution);
        Some(Self {
            settings: SourceSettings {
                macro_resolution: resolution,
                second_octave_offset: gradients.second_octave_offset,
                pixel_centered: u32::from(earth_data.is_some()),
                frequencies: octaves.map(|(frequency, _)| frequency),
            },
            bounds,
            earth,
            gradients: gradients.buffer,
            dem,
            climate,
        })
    }
}

pub(super) fn initialized<T: Pod>(
    device: &RenderDevice,
    label: &str,
    usage: BufferUsages,
    data: &[T],
) -> Buffer {
    device.create_buffer_with_data(&BufferInitDescriptor {
        label: Some(label),
        contents: cast_slice(data),
        usage,
    })
}

fn upload_texture<T: Pod>(
    device: &RenderDevice,
    queue: &RenderQueue,
    label: &str,
    size: [u32; 3],
    format: TextureFormat,
    data: &[T],
) -> TextureView {
    device
        .create_texture_with_data(
            queue,
            &TextureDescriptor {
                label: Some(label),
                size: Extent3d {
                    width: size[0],
                    height: size[1],
                    depth_or_array_layers: size[2],
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: TextureDimension::D2,
                format,
                usage: TextureUsages::TEXTURE_BINDING | TextureUsages::COPY_DST,
                view_formats: &[],
            },
            TextureDataOrder::LayerMajor,
            cast_slice(data),
        )
        .create_view(&TextureViewDescriptor::default())
}

fn upload_bounds(device: &RenderDevice, data: &SurfaceData, resolution: u32) -> Buffer {
    let mut bounds = Vec::new();
    for face in CubeFace::ALL {
        for lod in 0..=u8::try_from(resolution.trailing_zeros()).unwrap_or(0) {
            let n = 1 << lod;
            for y in 0..n {
                for x in 0..n {
                    bounds.push(
                        data.height_bounds(TileId { face, lod, x, y })
                            .map(meters_f32),
                    );
                }
            }
        }
    }
    initialized(
        device,
        "Terrain conservative bounds",
        BufferUsages::STORAGE,
        &bounds,
    )
}

pub(super) fn vec4_array(v: EcefMeters, w: f64) -> [f32; 4] {
    [v.x, v.y, v.z, w].map(meters_f32)
}

#[expect(
    clippy::cast_possible_truncation,
    reason = "GPU surface coefficients and camera-relative measurements are f32"
)]
pub(super) fn meters_f32(value: f64) -> f32 {
    value as f32
}

pub(super) fn grid_indices(segments: u32) -> Vec<u32> {
    let mut indices = Vec::new();
    let stride = segments + 1;
    for y in 0..segments {
        for x in 0..segments {
            let lower_left = y * stride + x;
            let lower_right = lower_left + 1;
            let upper_left = lower_left + stride;
            let upper_right = upper_left + 1;
            indices.extend_from_slice(&[
                lower_left,
                lower_right,
                upper_right,
                lower_left,
                upper_right,
                upper_left,
            ]);
        }
    }
    indices
}
