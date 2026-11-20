use crate::earth_pack::{invalid, unpack};
use crate::{
    LakeDef, LakeId, Navigability, RiverDef, RiverId, RiverWaypoint, Salinity, TerrainError,
    WaterBodyKind, WaterSample, lat_lon_to_unit,
};

const CATALOG: &str = include_str!("../data/earth/water.tsv");
const PROFILES: &[u8] = include_bytes!("../data/earth/rivers.zst");

#[derive(Debug, Clone, Copy)]
struct Entry {
    kind: WaterBodyKind,
    saline: bool,
    navigability: Navigability,
    name: &'static str,
}

#[derive(Debug)]
pub(crate) struct WaterCatalog {
    entries: Vec<Entry>,
    pub(crate) lakes: Vec<LakeDef>,
    pub(crate) rivers: Vec<RiverDef>,
}

impl WaterCatalog {
    pub(crate) fn embedded() -> Result<Self, TerrainError> {
        let mut entries = Vec::new();
        let mut lakes = Vec::new();
        for row in CATALOG.lines() {
            let fields: Vec<_> = row.splitn(10, '\t').collect();
            let [slot, id, kind, level, lon, lat, radius, depth, nav, name] = fields.as_slice()
            else {
                return Err(invalid("incomplete Earth water catalog record"));
            };
            if slot.parse::<usize>().ok() != Some(entries.len()) {
                return Err(invalid("non-sequential Earth water catalog slot"));
            }
            let id = id
                .parse()
                .map_err(|_| invalid("invalid Earth water identifier"))?;
            let kind = match *kind {
                "0" => WaterBodyKind::Dry,
                "1" => WaterBodyKind::Ocean,
                "2" | "3" => {
                    let saline = *kind == "3";
                    lakes.push(LakeDef {
                        id: LakeId(id),
                        name,
                        center_unit: lat_lon_to_unit(number(lat)?, number(lon)?),
                        radius_rad: number(radius)?.to_radians(),
                        surface_elevation_m: number(level)?,
                        max_depth_m: number(depth)?,
                        salinity: if saline {
                            Salinity::Saline
                        } else {
                            Salinity::Freshwater
                        },
                    });
                    WaterBodyKind::Lake(LakeId(id))
                }
                "4" => WaterBodyKind::River(RiverId(id)),
                _ => return Err(invalid("unknown Earth water kind")),
            };
            entries.push(Entry {
                kind,
                saline: matches!(kind, WaterBodyKind::Ocean) || fields[2] == "3",
                navigability: match *nav {
                    "0" => Navigability::Unnavigable,
                    "1" => Navigability::Barge,
                    "2" => Navigability::RiverVessel,
                    _ => return Err(invalid("unknown Earth navigability class")),
                },
                name,
            });
        }
        let rivers = decode_rivers(&entries)?;
        Ok(Self {
            entries,
            lakes,
            rivers,
        })
    }

    pub(crate) fn sample(&self, slot: usize, level: f64, height: f64) -> WaterSample {
        if level > height
            && let Some(entry) = self.entries.get(slot)
            && entry.kind != WaterBodyKind::Dry
        {
            return WaterSample {
                kind: entry.kind,
                surface_elevation_m: level,
                depth_m: level - height,
                discharge_m3s: 0.0,
                navigability: entry.navigability,
                is_freshwater: !entry.saline,
            };
        }
        dry(height)
    }
}

fn decode_rivers(entries: &[Entry]) -> Result<Vec<RiverDef>, TerrainError> {
    let profiles = unpack(PROFILES, 24 * 1024 * 1024)?;
    let (rows, remainder) = profiles.as_chunks::<16>();
    if !remainder.is_empty() {
        return Err(invalid("truncated Earth river profile"));
    }
    let mut rivers = Vec::new();
    let mut waypoints = Vec::new();
    let mut current = None;
    for row in rows {
        let values = row.as_chunks::<4>().0;
        let [index, lon, lat, elevation] = std::array::from_fn(|i| i32::from_le_bytes(values[i]));
        let index = usize::try_from(index).map_err(|_| invalid("invalid Earth river slot"))?;
        if current != Some(index) {
            if let Some(previous) = current {
                finish_river(entries, previous, &mut waypoints, &mut rivers)?;
            }
            current = Some(index);
        }
        waypoints.push(RiverWaypoint {
            unit_dir: lat_lon_to_unit(f64::from(lat) / 1e6, f64::from(lon) / 1e6),
            surface_elevation_m: f64::from(elevation),
            half_width_m: 230.0,
            channel_depth_m: 4.0,
            valley_half_width_m: 2_800.0,
            discharge_m3s: 0.0,
        });
    }
    if let Some(index) = current {
        finish_river(entries, index, &mut waypoints, &mut rivers)?;
    }
    Ok(rivers)
}

fn finish_river(
    entries: &[Entry],
    index: usize,
    waypoints: &mut Vec<RiverWaypoint>,
    rivers: &mut Vec<RiverDef>,
) -> Result<(), TerrainError> {
    let Some(entry) = entries.get(index) else {
        return Err(invalid("unknown Earth river slot"));
    };
    let WaterBodyKind::River(id) = entry.kind else {
        return Err(invalid("profile has no river catalog entry"));
    };
    rivers.push(RiverDef::new(
        id,
        entry.name,
        entry.navigability,
        std::mem::take(waypoints),
    )?);
    Ok(())
}

fn number(text: &str) -> Result<f64, TerrainError> {
    text.parse()
        .map_err(|_| invalid("invalid Earth water catalog number"))
}

pub(crate) const fn dry(height: f64) -> WaterSample {
    WaterSample {
        kind: WaterBodyKind::Dry,
        surface_elevation_m: height,
        depth_m: 0.0,
        discharge_m3s: 0.0,
        navigability: Navigability::Unnavigable,
        is_freshwater: true,
    }
}

pub(crate) fn dem_water(height: f64) -> WaterSample {
    if height < 0.0 {
        WaterSample {
            kind: WaterBodyKind::Ocean,
            surface_elevation_m: 0.0,
            depth_m: -height,
            discharge_m3s: 0.0,
            navigability: Navigability::Unnavigable,
            is_freshwater: false,
        }
    } else {
        dry(height)
    }
}
