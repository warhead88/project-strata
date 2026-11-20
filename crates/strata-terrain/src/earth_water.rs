use std::f64::consts::{PI, TAU};
use std::sync::{Arc, OnceLock};

use strata_globe::EcefMeters;

use crate::earth_pack::{invalid, read_rows};
use crate::water_catalog::WaterCatalog;
use crate::{LakeDef, RiverDef, TerrainError, WaterSample};

const INDEX: &[u8] = include_bytes!("../data/earth/water-index.zst");
const PAGES: &[u8] = include_bytes!("../data/earth/water-pages.zst");
pub(crate) const PAGE: usize = 16;
pub(crate) const STRIDE: usize = PAGE + 1;
pub(crate) const INDEX_WIDTH: usize = 5400;
pub(crate) const INDEX_HEIGHT: usize = 2700;
const PAGE_FLAG: u32 = 1 << 31;
static CACHE: OnceLock<Result<Arc<EarthWater>, TerrainError>> = OnceLock::new();

#[derive(Debug)]
pub(crate) struct EarthWater {
    index: Vec<u32>,
    cells: Vec<[i16; 4]>,
    catalog: WaterCatalog,
    page_bounds: Vec<(i16, i16, i16)>,
}

/// Immutable hydrology atlas, decoded at world load. The index holds either a
/// uniform water classification or a reference to a page with east/south guards.
#[derive(Clone, Copy)]
pub struct EarthWaterRaster<'a> {
    /// Page directory as a row-major raster, with the high bit marking a page reference.
    pub index: &'a [u32],
    /// Directory dimensions, in pages.
    pub index_size: [usize; 2],
    /// Texels in each page, including the guard row and column.
    pub page_stride: usize,
    /// Height correction, level, packed water classification/slot, reserved.
    pub cells: &'a [[i16; 4]],
}

#[derive(Clone, Copy)]
pub(crate) struct EarthWaterPoint {
    pub(crate) height_offset_m: f64,
    pub(crate) protection: f64,
    pub(crate) slot: usize,
    pub(crate) level_m: f64,
}

impl EarthWater {
    pub(crate) fn embedded() -> Result<Arc<Self>, TerrainError> {
        CACHE.get_or_init(|| Self::decode().map(Arc::new)).clone()
    }

    fn decode() -> Result<Self, TerrainError> {
        let mut index = Vec::with_capacity(INDEX_WIDTH * INDEX_HEIGHT);
        read_rows(INDEX, INDEX_WIDTH * 4, INDEX_HEIGHT, |row| {
            index.extend(
                row.as_chunks::<4>()
                    .0
                    .iter()
                    .map(|&v| u32::from_le_bytes(v)),
            );
        })?;
        let count = index
            .iter()
            .filter(|&&code| code & PAGE_FLAG != 0)
            .map(|&code| (code & !PAGE_FLAG) as usize + 1)
            .max()
            .ok_or_else(|| invalid("Earth water page directory is empty"))?;
        if count > INDEX_WIDTH * INDEX_HEIGHT {
            return Err(invalid(
                "Earth water page reference exceeds directory dimensions",
            ));
        }
        let mut cells = Vec::with_capacity(count * STRIDE * STRIDE);
        let mut page_bounds = Vec::with_capacity(count);
        read_rows(PAGES, STRIDE * STRIDE * 8, count, |page| {
            let mut bounds = (0_i16, 0_i16, i16::MIN);
            for &v in page.as_chunks::<8>().0 {
                let pairs = v.as_chunks::<2>().0;
                let cell: [i16; 4] = std::array::from_fn(|i| i16::from_le_bytes(pairs[i]));
                bounds.0 = bounds.0.min(cell[0]);
                bounds.1 = bounds.1.max(cell[0]);
                if cell[2] != 0 {
                    bounds.2 = bounds.2.max(cell[1]);
                }
                cells.push(cell);
            }
            page_bounds.push(bounds);
        })?;
        Ok(Self {
            index,
            cells,
            catalog: WaterCatalog::embedded()?,
            page_bounds,
        })
    }

    pub(crate) fn raster(&self) -> EarthWaterRaster<'_> {
        EarthWaterRaster {
            index: &self.index,
            index_size: [INDEX_WIDTH, INDEX_HEIGHT],
            page_stride: STRIDE,
            cells: &self.cells,
        }
    }

    pub(crate) fn page_bounds(&self, index: usize) -> (i16, i16, i16) {
        let code = self.index[index];
        if code & PAGE_FLAG != 0 {
            return self.page_bounds[(code & !PAGE_FLAG) as usize];
        }
        let level = match (code & 4095, code >> 16) {
            (0, _) => i16::MIN,
            (_, 0) => 0,
            (_, packed) => i16::try_from(packed).unwrap_or(16_384) - 16_384,
        };
        (0, 0, level)
    }

    pub(crate) fn lakes(&self) -> &[LakeDef] {
        &self.catalog.lakes
    }

    pub(crate) fn rivers(&self) -> &[RiverDef] {
        &self.catalog.rivers
    }

    pub(crate) fn water(&self, point: EarthWaterPoint, height_m: f64) -> WaterSample {
        self.catalog.sample(point.slot, point.level_m, height_m)
    }

    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "pixel coordinates are wrapped/clamped to the atlas dimensions"
    )]
    pub(crate) fn sample(&self, unit: EcefMeters) -> EarthWaterPoint {
        let lat = unit.y.atan2(unit.x.hypot(unit.z));
        let lon = unit.x.atan2(unit.z);
        let width = 86_400.0;
        let height = 43_200.0;
        let px = ((lon / TAU + 0.5) * width - 0.5).rem_euclid(width);
        let py = ((0.5 - lat / PI) * height - 0.5).clamp(0.0, height - 1.0);
        let x = px as usize;
        let y = py as usize;
        let code = self.index[y / PAGE * INDEX_WIDTH + x / PAGE];
        if code & PAGE_FLAG == 0 {
            return EarthWaterPoint {
                height_offset_m: 0.0,
                protection: f64::from(code & 4095 != 0),
                slot: code as usize & 4095,
                level_m: if code >> 16 == 0 {
                    0.0
                } else {
                    f64::from((code >> 16).cast_signed() - 16_384)
                },
            };
        }

        let base = (code & !PAGE_FLAG) as usize * STRIDE * STRIDE + y % PAGE * STRIDE + x % PAGE;
        let quad = [base, base + 1, base + STRIDE, base + STRIDE + 1].map(|i| self.cells[i]);
        let (tx, ty) = (px.fract(), py.fract());
        let weights = [
            (1.0 - tx) * (1.0 - ty),
            tx * (1.0 - ty),
            (1.0 - tx) * ty,
            tx * ty,
        ];
        let mut height_offset_m = 0.0;
        let mut wet = 0.0;
        let mut protection = 0.0;
        let mut best = -1.0;
        let mut slot = 0;
        let mut level = 0.0;
        for (cell, w) in quad.into_iter().zip(weights) {
            height_offset_m += f64::from(cell[0]) * w;
            let id = usize::from(cell[2].cast_unsigned() & 4095);
            if id != 0 || cell[0] != 0 {
                protection += w;
            }
            if id != 0 {
                wet += w;
                level += f64::from(cell[1]) * w;
                if w > best {
                    best = w;
                    slot = id;
                }
            }
        }
        EarthWaterPoint {
            height_offset_m,
            protection,
            slot: if wet >= 0.5 { slot } else { 0 },
            level_m: if wet > 0.0 { level / wet } else { 0.0 },
        }
    }
}
