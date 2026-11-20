use bevy::render::{
    render_resource::{
        AddressMode, Extent3d, FilterMode, Origin3d, Sampler, SamplerDescriptor,
        TexelCopyBufferLayout, TexelCopyTextureInfo, Texture, TextureAspect, TextureDescriptor,
        TextureDimension, TextureFormat, TextureUsages, TextureView, TextureViewDescriptor,
        TextureViewDimension,
    },
    renderer::{RenderDevice, RenderQueue},
};
use bytemuck::{Pod, cast_slice};
use strata_terrain::{EarthLocalRaster, EarthWaterRaster};

const RELIEF_SIDE: usize = 5400;
const PAGES_PER_LAYER: usize = 4096;

#[derive(Clone)]
pub(super) struct EarthTextures {
    pub(super) relief: TextureView,
    pub(super) directory: TextureView,
    pub(super) pages: TextureView,
    pub(super) amplitudes: TextureView,
    pub(super) shapes: TextureView,
    pub(super) amplitude_sampler: Sampler,
    pub(super) shape_sampler: Sampler,
}

impl EarthTextures {
    pub(super) fn upload(
        device: &RenderDevice,
        queue: &RenderQueue,
        data: Option<(&[i16], EarthWaterRaster<'_>, EarthLocalRaster<'_>)>,
    ) -> Option<Self> {
        let sampler = |address_mode_v| {
            device.create_sampler(&SamplerDescriptor {
                address_mode_u: AddressMode::Repeat,
                address_mode_v,
                mag_filter: FilterMode::Linear,
                min_filter: FilterMode::Linear,
                ..Default::default()
            })
        };
        let amplitude_sampler = sampler(AddressMode::ClampToEdge);
        let shape_sampler = sampler(AddressMode::Repeat);
        let Some((relief, water, local)) = data else {
            return Some(Self::empty(device, amplitude_sampler, shape_sampler));
        };

        let side = u32::try_from(RELIEF_SIDE).ok()?;
        let width = RELIEF_SIDE * 4;
        if relief.len() != width * RELIEF_SIDE * 2
            || side > device.limits().max_texture_dimension_2d
        {
            return None;
        }
        let elevation = texture(
            device,
            "Earth regional elevation",
            TextureFormat::R16Sint,
            [side, side, 8],
        );
        let mut layer_data = Vec::with_capacity(RELIEF_SIDE * RELIEF_SIDE);
        for (y, rows) in relief.chunks_exact(RELIEF_SIDE * width).enumerate() {
            for x in 0..4 {
                layer_data.clear();
                for row in rows.chunks_exact(width) {
                    layer_data.extend_from_slice(&row[x * RELIEF_SIDE..(x + 1) * RELIEF_SIDE]);
                }
                upload_layer(
                    queue,
                    &elevation,
                    [side, side],
                    u32::try_from(y * 4 + x).ok()?,
                    &layer_data,
                );
            }
            drain_uploads(device, queue)?;
        }
        drop(layer_data);

        let directory_size = water.index_size.map(|n| u32::try_from(n).unwrap_or(0));
        let directory = texture(
            device,
            "Earth water directory",
            TextureFormat::R32Uint,
            [directory_size[0], directory_size[1], 1],
        );
        upload_layer(queue, &directory, directory_size, 0, water.index);

        let stride = water.page_stride * water.page_stride;
        let pages = water.cells.len() / stride;
        let rows = PAGES_PER_LAYER.min(pages);
        let layers = u32::try_from(pages.div_ceil(rows)).ok()?;
        let width = u32::try_from(stride).ok()?;
        let pages = texture(
            device,
            "Earth water and terrain corrections",
            TextureFormat::Rgba16Sint,
            [width, u32::try_from(rows).ok()?, layers],
        );
        for (layer, cells) in water.cells.chunks(rows * stride).enumerate() {
            upload_layer(
                queue,
                &pages,
                [width, u32::try_from(cells.len() / stride).ok()?],
                u32::try_from(layer).ok()?,
                cells,
            );
            if (layer + 1).is_multiple_of(16) {
                drain_uploads(device, queue)?;
            }
        }
        drain_uploads(device, queue)?;
        let array_view = TextureViewDescriptor {
            dimension: Some(TextureViewDimension::D2Array),
            ..Default::default()
        };
        let (amplitudes, shapes) = upload_local(device, queue, local, &array_view)?;
        Some(Self {
            amplitudes,
            shapes,
            amplitude_sampler,
            shape_sampler,
            relief: elevation.create_view(&array_view),
            directory: directory.create_view(&TextureViewDescriptor::default()),
            pages: pages.create_view(&array_view),
        })
    }
    fn empty(device: &RenderDevice, amplitude_sampler: Sampler, shape_sampler: Sampler) -> Self {
        let make = |label, format, dimension| {
            texture(device, label, format, [1, 1, 1]).create_view(&TextureViewDescriptor {
                dimension: Some(dimension),
                ..Default::default()
            })
        };
        Self {
            relief: make(
                "Empty Earth relief",
                TextureFormat::R16Sint,
                TextureViewDimension::D2Array,
            ),
            directory: make(
                "Empty Earth directory",
                TextureFormat::R32Uint,
                TextureViewDimension::D2,
            ),
            amplitudes: make(
                "Empty Earth amplitudes",
                TextureFormat::Rg8Unorm,
                TextureViewDimension::D2,
            ),
            shapes: make(
                "Empty Earth landforms",
                TextureFormat::Rgba8Snorm,
                TextureViewDimension::D2Array,
            ),
            amplitude_sampler,
            shape_sampler,
            pages: make(
                "Empty Earth water",
                TextureFormat::Rgba16Sint,
                TextureViewDimension::D2Array,
            ),
        }
    }
}

fn upload_local(
    device: &RenderDevice,
    queue: &RenderQueue,
    local: EarthLocalRaster<'_>,
    array_view: &TextureViewDescriptor,
) -> Option<(TextureView, TextureView)> {
    let [aw, ah] = local.amplitude_size.map(|n| u32::try_from(n).unwrap_or(0));
    let amplitudes = texture(
        device,
        "Earth local amplitudes",
        TextureFormat::Rg8Unorm,
        [aw, ah, 1],
    );
    // Upload RG together, since upload_layer counts bytes per texel from T.
    upload_layer(
        queue,
        &amplitudes,
        [aw, ah],
        0,
        local.amplitudes.as_chunks::<2>().0,
    );
    let side = u32::try_from(local.shape_side).ok()?;
    let layers = u32::try_from(local.shape_layers).ok()?;
    let shapes = texture(
        device,
        "Earth local landforms",
        TextureFormat::Rgba8Snorm,
        [side, side, layers],
    );
    for (layer, sheet) in local
        .shapes
        .chunks_exact(local.shape_side.pow(2) * 4)
        .enumerate()
    {
        upload_layer(
            queue,
            &shapes,
            [side, side],
            u32::try_from(layer).ok()?,
            sheet.as_chunks::<4>().0,
        );
    }
    drain_uploads(device, queue)?;
    Some((
        amplitudes.create_view(&TextureViewDescriptor::default()),
        shapes.create_view(array_view),
    ))
}

fn texture(
    device: &RenderDevice,
    label: &str,
    format: TextureFormat,
    [width, height, depth_or_array_layers]: [u32; 3],
) -> Texture {
    device.create_texture(&TextureDescriptor {
        label: Some(label),
        format,
        size: Extent3d {
            width,
            height,
            depth_or_array_layers,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: TextureDimension::D2,
        usage: TextureUsages::TEXTURE_BINDING | TextureUsages::COPY_DST,
        view_formats: &[],
    })
}

fn upload_layer<T: Pod>(
    queue: &RenderQueue,
    texture: &Texture,
    [width, height]: [u32; 2],
    z: u32,
    data: &[T],
) {
    let texel_bytes = u32::try_from(size_of::<T>()).unwrap_or(0);
    queue.write_texture(
        TexelCopyTextureInfo {
            texture,
            mip_level: 0,
            origin: Origin3d { x: 0, y: 0, z },
            aspect: TextureAspect::All,
        },
        cast_slice(data),
        TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(width * texel_bytes),
            rows_per_image: Some(height),
        },
        Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
    );
}

fn drain_uploads(device: &RenderDevice, queue: &RenderQueue) -> Option<()> {
    // Release Metal's staging allocations in batches during source preparation.
    queue.submit([]);
    device
        .poll(wgpu::PollType::wait_indefinitely())
        .ok()
        .map(|_| ())
}
