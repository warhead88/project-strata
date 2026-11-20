use std::f64::consts::{PI, TAU};

use strata_globe::EcefMeters;

use crate::TerrainError;
use crate::earth_pack::{invalid, unpack};

const CONTROL: &[u8] = include_bytes!("../data/earth/local-amplitudes.zst");
const SHAPES: &[u8] = include_bytes!("../data/earth/local-shapes.zst");
const CONTROL_SIZE: [usize; 2] = [2700, 1350];
const SHAPE_SIDE: usize = 256;
const SHAPE_LAYERS: usize = 4;

// Hill and plain weights sum to at most one, including after interpolation.
pub(crate) const MAX_DISPLACEMENT_M: f64 = 123.0;
/// Largest period in the prepared Cartesian landform sheets, in meters.
pub const EARTH_LOCAL_PERIOD_M: f64 = 8192.0;

/// Prepared local relief and material. Amplitude texels are in equirectangular order.
/// Each sheet stores signed height, two slopes per texel, and material variation.
#[derive(Clone, Copy)]
pub struct EarthLocalRaster<'a> {
    /// Hill and plain amplitude channel pairs.
    pub amplitudes: &'a [u8],
    /// Width and height of the amplitude grid.
    pub amplitude_size: [usize; 2],
    /// Signed-normalized RGBA texels for hills, plains, ledges and ground material.
    pub shapes: &'a [u8],
    /// Square sheet side in texels.
    pub shape_side: usize,
    /// Number of sheets.
    pub shape_layers: usize,
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct EarthLocal {
    amplitudes: Vec<u8>,
    shapes: Vec<u8>,
}

impl EarthLocal {
    pub(crate) fn decode() -> Result<Self, TerrainError> {
        let control_size = CONTROL_SIZE[0] * CONTROL_SIZE[1] * 2;
        let shape_size = SHAPE_SIDE * SHAPE_SIDE * SHAPE_LAYERS * 4;
        let amplitudes = unpack(CONTROL, control_size as u64)?;
        let shapes = unpack(SHAPES, shape_size as u64)?;
        if amplitudes.len() != control_size || shapes.len() != shape_size {
            return Err(invalid("invalid local Earth relief dimensions"));
        }
        // Independent quantization can round either component upward. The baker
        // encodes hill and plain weights with a combined ceiling of 255.
        if amplitudes
            .as_chunks::<2>()
            .0
            .iter()
            .any(|&[hill, plain]| hill.checked_add(plain).is_none())
        {
            return Err(invalid(
                "Earth relief amplitudes exceed the height envelope",
            ));
        }
        Ok(Self { amplitudes, shapes })
    }

    pub(crate) fn raster(&self) -> EarthLocalRaster<'_> {
        EarthLocalRaster {
            amplitudes: &self.amplitudes,
            amplitude_size: CONTROL_SIZE,
            shapes: &self.shapes,
            shape_side: SHAPE_SIDE,
            shape_layers: SHAPE_LAYERS,
        }
    }

    pub(crate) fn sample(&self, unit: EcefMeters, radius_m: f64) -> (f64, EcefMeters) {
        let lon = unit.x.atan2(unit.z);
        let lat = unit.y.atan2(unit.x.hypot(unit.z));
        let x = (lon / TAU + 0.5) * 2700.0 - 0.5;
        let y = (0.5 - lat / PI) * 1350.0 - 0.5;
        let [hill, plain] = bilinear(x, y, CONTROL_SIZE, false, |x, y| {
            let i = (y * CONTROL_SIZE[0] + x) * 2;
            [
                f64::from(self.amplitudes[i]) / 255.0,
                f64::from(self.amplitudes[i + 1]) / 255.0,
            ]
        });
        let p = unit.scale(radius_m);
        let mut height = 0.0;
        let mut slope = EcefMeters::from_raw(0.0, 0.0, 0.0);
        for (layer, amplitude, step) in [
            (0, hill * 120.0, 32.0),
            (1, plain * 12.0, 32.0),
            (2, (hill + plain) * 3.0, 2.0),
        ] {
            if amplitude > 0.0 {
                let (h, grade) = self.project(p, unit, layer, step);
                height += h * amplitude;
                slope = slope + grade.scale(amplitude / step);
            }
        }
        (height, slope)
    }

    fn project(
        &self,
        position: EcefMeters,
        unit: EcefMeters,
        layer: usize,
        step: f64,
    ) -> (f64, EcefMeters) {
        let sample = |x, y| {
            bilinear(x, y, [SHAPE_SIDE; 2], true, |x, y| {
                let i = ((layer * SHAPE_SIDE + y) * SHAPE_SIDE + x) * 4;
                let signed = |channel| {
                    (f64::from(i8::from_ne_bytes([self.shapes[i + channel]])) / 127.0).max(-1.0)
                };
                [signed(0), signed(1), signed(2)]
            })
        };
        let [x, y, z] = [position.x / step, position.y / step, position.z / step];
        let [height_yz, grade_yz_y, grade_yz_z] = sample(y + 71.0, z + 113.0);
        let [height_zx, grade_zx_z, grade_zx_x] = sample(z + 13.0, x + 41.0);
        let [height_xy, grade_xy_x, grade_xy_y] = sample(x, y);
        let [wx, wy, wz] = [unit.x * unit.x, unit.y * unit.y, unit.z * unit.z];
        (
            height_yz * wx + height_zx * wy + height_xy * wz,
            EcefMeters::from_raw(
                grade_zx_x * wy + grade_xy_x * wz,
                grade_yz_y * wx + grade_xy_y * wz,
                grade_yz_z * wx + grade_zx_z * wy,
            ),
        )
    }
}

#[expect(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "atlas indices are wrapped or clamped into dimensions at most 5400"
)]
fn bilinear<const N: usize>(
    x: f64,
    y: f64,
    [width, height]: [usize; 2],
    repeat_y: bool,
    texel: impl Fn(usize, usize) -> [f64; N],
) -> [f64; N] {
    let x = x.rem_euclid(width as f64);
    let y = if repeat_y {
        y.rem_euclid(height as f64)
    } else {
        y.clamp(0.0, (height - 1) as f64)
    };
    let [col, row] = [x as usize % width, y as usize % height];
    let [nx, ny] = [
        (col + 1) % width,
        if repeat_y {
            (row + 1) % height
        } else {
            (row + 1).min(height - 1)
        },
    ];
    let [nw, ne, sw, se] = [
        texel(col, row),
        texel(nx, row),
        texel(col, ny),
        texel(nx, ny),
    ];
    std::array::from_fn(|k| {
        let top = nw[k] + (ne[k] - nw[k]) * x.fract();
        let bottom = sw[k] + (se[k] - sw[k]) * x.fract();
        top + (bottom - top) * y.fract()
    })
}

#[cfg(test)]
mod tests {
    use strata_globe::ElevationSampler;

    use super::*;
    use crate::{TerrainWorld, WaterBodyKind, earth_dem::EarthDem, lat_lon_to_unit};

    #[test]
    fn local_relief_reshapes_dry_ground_and_preserves_hydrology() -> Result<(), TerrainError> {
        let world = TerrainWorld::earth_default();
        let dem = EarthDem::global_embedded()?;
        for (lat, lon) in [(46.5406, 10.121), (33.4505, 45.0)] {
            let offsets = (0..=400).map(|i| {
                let unit = lat_lon_to_unit(lat + f64::from(i) * 0.000_001, lon);
                assert_eq!(world.sample_water(unit).kind, WaterBodyKind::Dry);
                let height = world.sample_elevation_unit_prevalidated(unit);
                height - dem.sample_bilinear(unit)
            });
            let mut low = f64::INFINITY;
            let mut high = f64::NEG_INFINITY;
            let mut previous: Option<f64> = None;
            for offset in offsets {
                low = low.min(offset);
                high = high.max(offset);
                assert!(offset.abs() <= MAX_DISPLACEMENT_M);
                if let Some(last) = previous {
                    assert!((offset - last).abs() < 0.20);
                }
                previous = Some(offset);
            }
            assert!(high - low > 0.35, "{lat}, {lon}: {low} .. {high}");
        }

        let mut protected = 0;
        for (lat, lon) in [(45.289, 36.575), (53.17, 107.125), (30.01, 31.25)] {
            for i in -30..=30 {
                let unit = lat_lon_to_unit(lat, lon + f64::from(i) * 0.0005);
                let point = world
                    .earth_water
                    .as_ref()
                    .ok_or_else(|| invalid("missing atlas"))?
                    .sample(unit);
                if point.protection >= 0.35 {
                    let expected = dem.sample_bilinear(unit) + point.height_offset_m;
                    assert!((world.sample_elevation(unit) - expected).abs() < 1e-8);
                    protected += 1;
                }
            }
        }
        assert!(protected > 25);
        let qattara_floor = lat_lon_to_unit(30.0, 27.5);
        assert_eq!(world.sample_water(qattara_floor).kind, WaterBodyKind::Dry);
        assert!(world.sample_elevation(qattara_floor) < -40.0);
        let at = |lat, lon| {
            dem.local
                .sample(lat_lon_to_unit(lat, lon), world.planet().radius_meters())
                .0
        };
        assert!((at(65.0, 179.999_999) - at(65.0, -179.999_999)).abs() < 0.02);
        assert!(at(89.999, 102.0).abs() < f64::EPSILON);
        assert!(at(-89.999, -42.0).abs() < f64::EPSILON);
        Ok(())
    }
}
