use std::f64::consts::{FRAC_PI_2, PI, TAU};

use strata_globe::{CubeFace, EcefMeters};

use crate::TerrainError;
use crate::earth_pack::read_rows;
use crate::earth_water::{EarthWater, INDEX_HEIGHT, INDEX_WIDTH};

const BLOCKS_X: usize = INDEX_WIDTH / 2;
const BLOCKS_Y: usize = INDEX_HEIGHT / 2;
const PACKED: &[u8] = include_bytes!("../data/earth/relief-bounds.zst");

pub(crate) fn scatter(
    resolution: u32,
    water: &EarthWater,
    low: &mut [f64],
    high: &mut [f64],
) -> Result<(), TerrainError> {
    let dim = resolution as usize;
    let longitudes = axes(BLOCKS_X, -PI, TAU);
    let latitudes = axes(BLOCKS_Y, FRAC_PI_2, -PI);
    let sin_cos = CubeFace::ALL.map(CubeFace::basis);
    let mut row_index = 0;
    // Chart angles are atan2 of projections onto two Cartesian axes. Inside this
    // cap the projection length is >=0.57, so the angle moves < 1.8 * cap radius.
    let cap = (2.0_f64 / 15.0).to_radians();
    let spread = 1.8 * cap / FRAC_PI_2;
    read_rows(PACKED, BLOCKS_X * 4, BLOCKS_Y, |bytes| {
        let (sl, cl) = latitudes[row_index];
        for (col, &[a, b, c, d]) in bytes.as_chunks::<4>().0.iter().enumerate() {
            let (sx, cx) = longitudes[col];
            let center = EcefMeters::from_raw(cl * sx, sl, cl * cx);
            let dominant = center.x.abs().max(center.y.abs()).max(center.z.abs());
            let base_min = f64::from(i16::from_le_bytes([a, b]));
            let base_max = f64::from(i16::from_le_bytes([c, d]));
            let index = row_index * 2 * INDEX_WIDTH + col * 2;
            let bounds = [
                index,
                index + 1,
                index + INDEX_WIDTH,
                index + INDEX_WIDTH + 1,
            ]
            .map(|page| water.page_bounds(page));
            let (offset_min, offset_max, water_max) = bounds
                .into_iter()
                .fold((0, 0, i16::MIN), |(lo, hi, level), (a, b, c)| {
                    (lo.min(a), hi.max(b), level.max(c))
                });
            let min_m = base_min + f64::from(offset_min);
            // Both renderers add the sea-level shell for culling. Keep abyssal
            // tile bounds tight and extend only for positive inland water levels.
            let bed_max = base_max + f64::from(offset_max);
            let max_m = if water_max > 0 {
                bed_max.max(f64::from(water_max))
            } else {
                bed_max
            };
            for (face, basis) in sin_cos.iter().enumerate() {
                let normal = center.dot(basis.normal);
                if normal + 2.0 * cap < dominant {
                    continue;
                }
                let u = 0.5 + center.dot(basis.right).atan2(normal) / FRAC_PI_2;
                let v = 0.5 + center.dot(basis.up).atan2(normal) / FRAC_PI_2;
                for y in grid_cell(v - spread, resolution)..=grid_cell(v + spread, resolution) {
                    for x in grid_cell(u - spread, resolution)..=grid_cell(u + spread, resolution) {
                        let i = (face * dim + y) * dim + x;
                        low[i] = low[i].min(min_m);
                        high[i] = high[i].max(max_m);
                    }
                }
            }
        }
        row_index += 1;
    })
}

#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "coordinate is clamped to the cube face, with resolution at most 512"
)]
fn grid_cell(uv: f64, resolution: u32) -> usize {
    ((uv.clamp(0.0, 1.0) * f64::from(resolution)) as usize).min(resolution as usize - 1)
}

#[expect(
    clippy::cast_precision_loss,
    reason = "Earth page dimensions are at most 2700"
)]
fn axes(count: usize, start: f64, span: f64) -> Vec<(f64, f64)> {
    (0..count)
        .map(|i| (start + span * (i as f64 + 0.5) / count as f64).sin_cos())
        .collect()
}
