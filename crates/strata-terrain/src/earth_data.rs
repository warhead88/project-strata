/// Geology province identifiers stored in `CubeMacroCache` and exposed via `TerrainWorld`.
pub const PROVINCE_OCEANIC_ABYSS: u8 = 0;
/// Continental shelf and passive margin.
pub const PROVINCE_CONTINENTAL_SHELF: u8 = 1;
/// Stable Precambrian craton / shield.
pub const PROVINCE_CRATON: u8 = 2;
/// Sedimentary basin / lowland plains.
pub const PROVINCE_SEDIMENTARY_BASIN: u8 = 3;
/// Young Cenozoic/Mesozoic orogenic mountain belt.
pub const PROVINCE_YOUNG_OROGEN: u8 = 4;
/// Ancient Paleozoic orogen or continental rift.
pub const PROVINCE_OLD_OROGEN_RIFT: u8 = 5;
/// Subduction trench / volcanic island arc.
pub const PROVINCE_SUBDUCTION_ARC: u8 = 6;
/// Polar ice-capped shield.
pub const PROVINCE_POLAR_SHIELD: u8 = 7;

/// Lightweight lat/lon bounding box (degrees) for classifying major Earth cratons and subduction arcs.
#[derive(Debug, Clone, Copy)]
pub(crate) struct GeoRegionBox {
    pub lat_min: f64,
    pub lat_max: f64,
    pub lon_min: f64,
    pub lon_max: f64,
}

impl GeoRegionBox {
    #[inline]
    #[must_use]
    pub(crate) fn contains(self, lat_deg: f64, lon_deg: f64) -> bool {
        lat_deg >= self.lat_min
            && lat_deg <= self.lat_max
            && lon_deg >= self.lon_min
            && lon_deg <= self.lon_max
    }
}

/// Major Precambrian craton shields (used where terrain is low-to-moderate relief).
pub(crate) const CRATON_REGIONS: &[GeoRegionBox] = &[
    // Fennoscandian / Baltic Shield
    GeoRegionBox {
        lat_min: 57.0,
        lat_max: 71.0,
        lon_min: 5.0,
        lon_max: 40.0,
    },
    // Canadian Shield
    GeoRegionBox {
        lat_min: 46.0,
        lat_max: 72.0,
        lon_min: -115.0,
        lon_max: -56.0,
    },
    // Siberian Craton
    GeoRegionBox {
        lat_min: 55.0,
        lat_max: 73.0,
        lon_min: 85.0,
        lon_max: 135.0,
    },
    // West & Southern African Cratons
    GeoRegionBox {
        lat_min: 10.0,
        lat_max: 28.0,
        lon_min: -16.0,
        lon_max: 28.0,
    },
    GeoRegionBox {
        lat_min: -32.0,
        lat_max: -12.0,
        lon_min: 14.0,
        lon_max: 34.0,
    },
    // Arabian Shield
    GeoRegionBox {
        lat_min: 14.0,
        lat_max: 31.0,
        lon_min: 35.0,
        lon_max: 56.0,
    },
    // Indian Shield
    GeoRegionBox {
        lat_min: 8.0,
        lat_max: 25.0,
        lon_min: 72.0,
        lon_max: 86.0,
    },
    // Brazilian Shield
    GeoRegionBox {
        lat_min: -24.0,
        lat_max: -4.0,
        lon_min: -56.0,
        lon_max: -36.0,
    },
    // Australian Craton
    GeoRegionBox {
        lat_min: -34.0,
        lat_max: -14.0,
        lon_min: 114.0,
        lon_max: 144.0,
    },
];

/// Major Pacific / Indonesian subduction island arcs & trenches.
pub(crate) const SUBDUCTION_ARC_REGIONS: &[GeoRegionBox] = &[
    // Japan / Kuril / Kamchatka
    GeoRegionBox {
        lat_min: 30.0,
        lat_max: 58.0,
        lon_min: 129.0,
        lon_max: 164.0,
    },
    // Sumatra / Java / Sunda / Philippines
    GeoRegionBox {
        lat_min: -11.0,
        lat_max: 20.0,
        lon_min: 95.0,
        lon_max: 128.0,
    },
    // New Zealand / Tonga-Kermadec
    GeoRegionBox {
        lat_min: -47.0,
        lat_max: -14.0,
        lon_min: 166.0,
        lon_max: 180.0,
    },
    // Aleutians
    GeoRegionBox {
        lat_min: 51.0,
        lat_max: 57.0,
        lon_min: -180.0,
        lon_max: -158.0,
    },
];
