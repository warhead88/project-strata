use bevy::asset::RenderAssetUsages;
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use strata_globe::EcefMeters;

use crate::camera::{GlobeCameraState, OriginState};
use crate::globe_render::GlobeRenderState;
use crate::snapshot::ClientWorldView;
use crate::terrain::{TerrainElevation, sample_clamped_elevation};

/// Draws great-circle cube-face edge highlights using `Gizmos` when `debug_overlay` is active.
#[expect(
    clippy::needless_pass_by_value,
    reason = "Bevy ECS system parameters require Res<T> by value"
)]
pub fn draw_cube_face_seams(
    mut gizmos: Gizmos,
    world_view: Res<ClientWorldView>,
    terrain: Res<TerrainElevation>,
    cam_state: Res<GlobeCameraState>,
    origin_state: Res<OriginState>,
    render_state: Res<GlobeRenderState>,
) {
    if !render_state.debug_overlay {
        return;
    }
    let planet = world_view.snapshot().planet;
    let r = planet.radius_meters();
    let min_r = 1.0_f64.min(r * 0.5);
    let alt = cam_state.altitude_meters.max(1.0);
    let origin = origin_state.origin;
    let sampler = terrain.sampler();
    let seam_color = Color::srgba(0.95, 0.78, 0.30, 0.55);

    let edges: [(EcefMeters, EcefMeters); 12] = [
        (
            EcefMeters::from_raw(1.0, -1.0, -1.0),
            EcefMeters::from_raw(1.0, 1.0, -1.0),
        ),
        (
            EcefMeters::from_raw(1.0, -1.0, 1.0),
            EcefMeters::from_raw(1.0, 1.0, 1.0),
        ),
        (
            EcefMeters::from_raw(-1.0, -1.0, -1.0),
            EcefMeters::from_raw(-1.0, 1.0, -1.0),
        ),
        (
            EcefMeters::from_raw(-1.0, -1.0, 1.0),
            EcefMeters::from_raw(-1.0, 1.0, 1.0),
        ),
        (
            EcefMeters::from_raw(-1.0, 1.0, -1.0),
            EcefMeters::from_raw(1.0, 1.0, -1.0),
        ),
        (
            EcefMeters::from_raw(-1.0, 1.0, 1.0),
            EcefMeters::from_raw(1.0, 1.0, 1.0),
        ),
        (
            EcefMeters::from_raw(-1.0, -1.0, -1.0),
            EcefMeters::from_raw(1.0, -1.0, -1.0),
        ),
        (
            EcefMeters::from_raw(-1.0, -1.0, 1.0),
            EcefMeters::from_raw(1.0, -1.0, 1.0),
        ),
        (
            EcefMeters::from_raw(-1.0, -1.0, 1.0),
            EcefMeters::from_raw(-1.0, -1.0, -1.0),
        ),
        (
            EcefMeters::from_raw(1.0, -1.0, 1.0),
            EcefMeters::from_raw(1.0, -1.0, -1.0),
        ),
        (
            EcefMeters::from_raw(-1.0, 1.0, 1.0),
            EcefMeters::from_raw(-1.0, 1.0, -1.0),
        ),
        (
            EcefMeters::from_raw(1.0, 1.0, 1.0),
            EcefMeters::from_raw(1.0, 1.0, -1.0),
        ),
    ];

    let focus = cam_state.focus_unit;
    let local_mode = alt < r * 0.02;
    let segments = 64_u32;
    let inv_seg = 1.0 / f64::from(segments);

    for (a, b) in edges {
        let dir = b - a;
        let (t_min, t_max, lift) = if local_mode {
            let focus_cube = focus.scale(1.0 / focus.x.abs().max(focus.y.abs()).max(focus.z.abs()));
            let t_proj = ((focus_cube - a).dot(dir) / dir.dot(dir)).clamp(0.0, 1.0);
            let half_window = ((alt * 12.0) / r).clamp(1e-6, 0.5);
            let lo = (t_proj - half_window).max(0.0);
            let hi = (t_proj + half_window).min(1.0);
            if let Ok(c_unit) = (a + dir.scale(t_proj)).normalized()
                && c_unit.distance_to(focus) * r > alt * 18.0
            {
                continue;
            }
            (lo, hi, (alt * 0.001).clamp(0.02, 10.0))
        } else {
            let seg_arc = planet.face_arc_length_meters() * inv_seg;
            let sag = (seg_arc * seg_arc) / (8.0 * r);
            (0.0, 1.0, sag + alt * 0.0015)
        };

        let mut prev = None;
        for step in 0..=segments {
            let t = t_min + (t_max - t_min) * (f64::from(step) * inv_seg);
            if let Ok(unit) = (a + dir.scale(t)).normalized() {
                let elev = sample_clamped_elevation(planet, unit, sampler);
                let pt = origin.to_render_vec3(unit.scale((r + elev).max(min_r) + lift));
                let v = Vec3::new(pt.x, pt.y, pt.z);
                if let Some(p) = prev {
                    gizmos.line(p, v, seam_color);
                }
                prev = Some(v);
            }
        }
    }
}

/// Builds the procedural tile grid texture used when `debug_overlay` is enabled.
#[must_use]
pub(crate) fn build_tile_grid_image() -> Image {
    let size = 128_u32;
    let mut data = Vec::with_capacity((size * size * 4) as usize);
    for y in 0..size {
        for x in 0..size {
            let is_outer_border = x <= 1 || y <= 1 || x >= size - 2 || y >= size - 2;
            let is_subgrid = (x % 32 == 0) || (y % 32 == 0);
            let shade: u8 = if is_outer_border {
                245
            } else if is_subgrid {
                205
            } else {
                172
            };
            data.extend_from_slice(&[shade, shade, shade, 255]);
        }
    }
    Image::new(
        Extent3d {
            width: size,
            height: size,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        data,
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::default(),
    )
}
