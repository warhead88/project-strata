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

/// Specification of a continental land lobe or marine basin in spherical coordinates.
#[derive(Debug, Clone, Copy)]
pub(crate) struct GeoLobeSpec {
    pub lat_deg: f64,
    pub lon_deg: f64,
    pub radius_u_deg: f64,
    pub radius_v_deg: f64,
    pub rot_deg: f64,
    /// Positive = continental uplift; negative = sea/bay carving.
    pub weight: f64,
    pub base_elev_m: f64,
    pub province: u8,
}

/// Specification of a linear mountain belt or deep ocean trench along a great-circle arc.
#[derive(Debug, Clone, Copy)]
pub(crate) struct GeoRidgeSpec {
    pub lat_a_deg: f64,
    pub lon_a_deg: f64,
    pub lat_b_deg: f64,
    pub lon_b_deg: f64,
    pub half_width_deg: f64,
    /// Positive = mountain belt uplift; negative = ocean trench depression.
    pub peak_elev_m: f64,
    pub roughness_boost: f64,
    pub province: u8,
}

/// Major Earth continents, islands, and inland seas/bays.
#[rustfmt::skip]
pub(crate) const EARTH_LOBES: &[GeoLobeSpec] = &[
    // --- Eurasia ---
    GeoLobeSpec { lat_deg: 50.0, lon_deg: 13.0, radius_u_deg: 11.0, radius_v_deg: 7.5, rot_deg: 15.0, weight: 1.05, base_elev_m: 320.0, province: PROVINCE_SEDIMENTARY_BASIN }, // Central Europe
    GeoLobeSpec { lat_deg: 40.5, lon_deg: -4.0, radius_u_deg: 5.0, radius_v_deg: 4.0, rot_deg: 0.0, weight: 0.95, base_elev_m: 580.0, province: PROVINCE_OLD_OROGEN_RIFT },  // Iberia
    GeoLobeSpec { lat_deg: 42.5, lon_deg: 12.5, radius_u_deg: 2.3, radius_v_deg: 5.5, rot_deg: -32.0, weight: 0.85, base_elev_m: 380.0, province: PROVINCE_YOUNG_OROGEN }, // Italy
    GeoLobeSpec { lat_deg: 63.0, lon_deg: 16.5, radius_u_deg: 6.5, radius_v_deg: 8.5, rot_deg: 25.0, weight: 1.0, base_elev_m: 420.0, province: PROVINCE_CRATON },        // Scandinavia
    GeoLobeSpec { lat_deg: 54.0, lon_deg: -2.0, radius_u_deg: 3.0, radius_v_deg: 4.2, rot_deg: -8.0, weight: 0.92, base_elev_m: 210.0, province: PROVINCE_SEDIMENTARY_BASIN }, // UK & Ireland
    GeoLobeSpec { lat_deg: 64.9, lon_deg: -18.5, radius_u_deg: 3.0, radius_v_deg: 2.0, rot_deg: 10.0, weight: 0.85, base_elev_m: 520.0, province: PROVINCE_SUBDUCTION_ARC }, // Iceland
    GeoLobeSpec { lat_deg: 56.0, lon_deg: 42.0, radius_u_deg: 18.0, radius_v_deg: 11.0, rot_deg: 8.0, weight: 1.25, base_elev_m: 220.0, province: PROVINCE_CRATON },      // Eastern Europe
    GeoLobeSpec { lat_deg: 62.0, lon_deg: 88.0, radius_u_deg: 26.0, radius_v_deg: 12.5, rot_deg: 4.0, weight: 1.35, base_elev_m: 360.0, province: PROVINCE_CRATON },      // Siberia
    GeoLobeSpec { lat_deg: 63.0, lon_deg: 142.0, radius_u_deg: 18.0, radius_v_deg: 9.0, rot_deg: -10.0, weight: 1.15, base_elev_m: 580.0, province: PROVINCE_OLD_OROGEN_RIFT }, // Russian Far East
    GeoLobeSpec { lat_deg: 56.0, lon_deg: 159.5, radius_u_deg: 3.2, radius_v_deg: 6.0, rot_deg: 22.0, weight: 0.85, base_elev_m: 650.0, province: PROVINCE_SUBDUCTION_ARC }, // Kamchatka
    GeoLobeSpec { lat_deg: 39.0, lon_deg: 35.0, radius_u_deg: 7.5, radius_v_deg: 4.0, rot_deg: 0.0, weight: 1.0, base_elev_m: 980.0, province: PROVINCE_YOUNG_OROGEN },   // Anatolia
    GeoLobeSpec { lat_deg: 33.0, lon_deg: 54.0, radius_u_deg: 11.0, radius_v_deg: 7.5, rot_deg: -18.0, weight: 1.15, base_elev_m: 1200.0, province: PROVINCE_YOUNG_OROGEN }, // Iran / Iranian Plateau
    GeoLobeSpec { lat_deg: 23.0, lon_deg: 45.0, radius_u_deg: 9.5, radius_v_deg: 8.5, rot_deg: -25.0, weight: 1.15, base_elev_m: 680.0, province: PROVINCE_CRATON },      // Arabian Peninsula
    GeoLobeSpec { lat_deg: 43.0, lon_deg: 68.0, radius_u_deg: 15.0, radius_v_deg: 9.0, rot_deg: 5.0, weight: 1.2, base_elev_m: 540.0, province: PROVINCE_SEDIMENTARY_BASIN }, // Central Asia
    GeoLobeSpec { lat_deg: 32.5, lon_deg: 87.5, radius_u_deg: 14.0, radius_v_deg: 8.5, rot_deg: -6.0, weight: 1.30, base_elev_m: 4600.0, province: PROVINCE_YOUNG_OROGEN }, // Tibetan Plateau
    GeoLobeSpec { lat_deg: 20.5, lon_deg: 78.5, radius_u_deg: 8.0, radius_v_deg: 10.0, rot_deg: 0.0, weight: 1.12, base_elev_m: 480.0, province: PROVINCE_CRATON },       // India
    GeoLobeSpec { lat_deg: 35.5, lon_deg: 109.0, radius_u_deg: 16.0, radius_v_deg: 11.5, rot_deg: 12.0, weight: 1.3, base_elev_m: 620.0, province: PROVINCE_SEDIMENTARY_BASIN }, // China
    GeoLobeSpec { lat_deg: 16.0, lon_deg: 102.0, radius_u_deg: 6.5, radius_v_deg: 8.5, rot_deg: -12.0, weight: 1.05, base_elev_m: 380.0, province: PROVINCE_OLD_OROGEN_RIFT }, // Indochina
    GeoLobeSpec { lat_deg: 36.5, lon_deg: 138.0, radius_u_deg: 3.2, radius_v_deg: 7.0, rot_deg: 36.0, weight: 0.92, base_elev_m: 620.0, province: PROVINCE_SUBDUCTION_ARC }, // Japan
    GeoLobeSpec { lat_deg: 0.0, lon_deg: 101.5, radius_u_deg: 3.5, radius_v_deg: 7.0, rot_deg: -40.0, weight: 0.88, base_elev_m: 350.0, province: PROVINCE_SUBDUCTION_ARC }, // Sumatra
    GeoLobeSpec { lat_deg: -7.5, lon_deg: 110.5, radius_u_deg: 5.0, radius_v_deg: 2.0, rot_deg: -5.0, weight: 0.85, base_elev_m: 360.0, province: PROVINCE_SUBDUCTION_ARC }, // Java
    GeoLobeSpec { lat_deg: 0.5, lon_deg: 114.5, radius_u_deg: 4.5, radius_v_deg: 4.5, rot_deg: 10.0, weight: 0.90, base_elev_m: 390.0, province: PROVINCE_SUBDUCTION_ARC }, // Borneo
    GeoLobeSpec { lat_deg: -4.5, lon_deg: 140.0, radius_u_deg: 7.0, radius_v_deg: 3.2, rot_deg: -20.0, weight: 0.90, base_elev_m: 620.0, province: PROVINCE_SUBDUCTION_ARC }, // New Guinea

    // --- Africa ---
    GeoLobeSpec { lat_deg: 22.0, lon_deg: 5.0, radius_u_deg: 21.0, radius_v_deg: 12.5, rot_deg: 4.0, weight: 1.35, base_elev_m: 440.0, province: PROVINCE_CRATON },       // North/West Africa (Sahara)
    GeoLobeSpec { lat_deg: 19.0, lon_deg: 27.0, radius_u_deg: 12.0, radius_v_deg: 11.0, rot_deg: 0.0, weight: 1.25, base_elev_m: 480.0, province: PROVINCE_CRATON },      // Northeast Africa
    GeoLobeSpec { lat_deg: 8.0, lon_deg: 43.0, radius_u_deg: 6.5, radius_v_deg: 6.5, rot_deg: 20.0, weight: 1.05, base_elev_m: 950.0, province: PROVINCE_OLD_OROGEN_RIFT }, // Horn of Africa
    GeoLobeSpec { lat_deg: 0.0, lon_deg: 23.0, radius_u_deg: 12.5, radius_v_deg: 11.0, rot_deg: 0.0, weight: 1.25, base_elev_m: 450.0, province: PROVINCE_SEDIMENTARY_BASIN }, // Central Africa (Congo)
    GeoLobeSpec { lat_deg: -20.0, lon_deg: 24.0, radius_u_deg: 10.0, radius_v_deg: 13.0, rot_deg: 5.0, weight: 1.2, base_elev_m: 980.0, province: PROVINCE_CRATON },    // Southern Africa
    GeoLobeSpec { lat_deg: -19.0, lon_deg: 46.5, radius_u_deg: 2.8, radius_v_deg: 6.5, rot_deg: 18.0, weight: 0.92, base_elev_m: 580.0, province: PROVINCE_CRATON },    // Madagascar

    // --- North America & Greenland ---
    GeoLobeSpec { lat_deg: 64.0, lon_deg: -152.0, radius_u_deg: 11.0, radius_v_deg: 6.0, rot_deg: 10.0, weight: 1.05, base_elev_m: 640.0, province: PROVINCE_YOUNG_OROGEN }, // Alaska
    GeoLobeSpec { lat_deg: 57.0, lon_deg: -102.0, radius_u_deg: 22.0, radius_v_deg: 12.5, rot_deg: -8.0, weight: 1.35, base_elev_m: 380.0, province: PROVINCE_CRATON }, // Canada
    GeoLobeSpec { lat_deg: 39.0, lon_deg: -98.0, radius_u_deg: 19.0, radius_v_deg: 10.5, rot_deg: 3.0, weight: 1.3, base_elev_m: 520.0, province: PROVINCE_SEDIMENTARY_BASIN }, // United States
    GeoLobeSpec { lat_deg: 21.0, lon_deg: -100.0, radius_u_deg: 6.5, radius_v_deg: 9.0, rot_deg: -35.0, weight: 1.0, base_elev_m: 1100.0, province: PROVINCE_YOUNG_OROGEN }, // Mexico / Central America
    GeoLobeSpec { lat_deg: 72.5, lon_deg: -42.0, radius_u_deg: 9.0, radius_v_deg: 10.5, rot_deg: 15.0, weight: 1.2, base_elev_m: 1950.0, province: PROVINCE_POLAR_SHIELD }, // Greenland

    // --- South America ---
    GeoLobeSpec { lat_deg: -4.0, lon_deg: -62.0, radius_u_deg: 14.0, radius_v_deg: 11.0, rot_deg: -10.0, weight: 1.3, base_elev_m: 220.0, province: PROVINCE_SEDIMENTARY_BASIN }, // Amazon
    GeoLobeSpec { lat_deg: -14.0, lon_deg: -47.0, radius_u_deg: 11.0, radius_v_deg: 10.5, rot_deg: 10.0, weight: 1.2, base_elev_m: 560.0, province: PROVINCE_CRATON },  // Brazil Highlands
    GeoLobeSpec { lat_deg: -36.0, lon_deg: -66.0, radius_u_deg: 6.5, radius_v_deg: 14.0, rot_deg: 12.0, weight: 1.1, base_elev_m: 480.0, province: PROVINCE_SEDIMENTARY_BASIN }, // Southern Cone

    // --- Australia & Oceania ---
    GeoLobeSpec { lat_deg: -25.0, lon_deg: 134.0, radius_u_deg: 17.0, radius_v_deg: 10.5, rot_deg: -6.0, weight: 1.25, base_elev_m: 340.0, province: PROVINCE_CRATON }, // Australia
    GeoLobeSpec { lat_deg: -42.0, lon_deg: 172.0, radius_u_deg: 2.5, radius_v_deg: 5.5, rot_deg: 32.0, weight: 0.88, base_elev_m: 620.0, province: PROVINCE_SUBDUCTION_ARC }, // New Zealand

    // --- Antarctica ---
    GeoLobeSpec { lat_deg: -83.0, lon_deg: 45.0, radius_u_deg: 24.0, radius_v_deg: 18.0, rot_deg: 0.0, weight: 1.4, base_elev_m: 2700.0, province: PROVINCE_POLAR_SHIELD }, // East Antarctica
    GeoLobeSpec { lat_deg: -78.0, lon_deg: -95.0, radius_u_deg: 12.0, radius_v_deg: 8.5, rot_deg: 15.0, weight: 1.15, base_elev_m: 1450.0, province: PROVINCE_POLAR_SHIELD }, // West Antarctica

    // --- Marine Basins & Seas (negative carving) ---
    GeoLobeSpec { lat_deg: 35.8, lon_deg: 16.0, radius_u_deg: 16.5, radius_v_deg: 4.5, rot_deg: -6.0, weight: -1.45, base_elev_m: -1600.0, province: PROVINCE_OCEANIC_ABYSS }, // Mediterranean Sea
    GeoLobeSpec { lat_deg: 43.3, lon_deg: 34.0, radius_u_deg: 5.5, radius_v_deg: 2.6, rot_deg: 0.0, weight: -1.4, base_elev_m: -1200.0, province: PROVINCE_OCEANIC_ABYSS }, // Black Sea
    GeoLobeSpec { lat_deg: 20.5, lon_deg: 38.5, radius_u_deg: 2.2, radius_v_deg: 8.5, rot_deg: -30.0, weight: -1.5, base_elev_m: -1100.0, province: PROVINCE_OLD_OROGEN_RIFT }, // Red Sea
    GeoLobeSpec { lat_deg: 26.8, lon_deg: 51.5, radius_u_deg: 4.2, radius_v_deg: 2.0, rot_deg: -35.0, weight: -1.25, base_elev_m: -75.0, province: PROVINCE_CONTINENTAL_SHELF }, // Persian Gulf
    GeoLobeSpec { lat_deg: 58.0, lon_deg: 20.0, radius_u_deg: 3.5, radius_v_deg: 5.5, rot_deg: 22.0, weight: -1.3, base_elev_m: -120.0, province: PROVINCE_CONTINENTAL_SHELF }, // Baltic Sea
    GeoLobeSpec { lat_deg: 60.0, lon_deg: -85.0, radius_u_deg: 6.5, radius_v_deg: 5.8, rot_deg: 0.0, weight: -1.45, base_elev_m: -160.0, province: PROVINCE_CONTINENTAL_SHELF }, // Hudson Bay
    GeoLobeSpec { lat_deg: 25.0, lon_deg: -90.0, radius_u_deg: 7.5, radius_v_deg: 5.5, rot_deg: 0.0, weight: -1.5, base_elev_m: -1800.0, province: PROVINCE_OCEANIC_ABYSS }, // Gulf of Mexico
    GeoLobeSpec { lat_deg: 40.0, lon_deg: 134.5, radius_u_deg: 4.0, radius_v_deg: 5.5, rot_deg: 30.0, weight: -1.4, base_elev_m: -1600.0, province: PROVINCE_OCEANIC_ABYSS }, // Sea of Japan
    GeoLobeSpec { lat_deg: 13.0, lon_deg: 114.0, radius_u_deg: 6.5, radius_v_deg: 7.5, rot_deg: 20.0, weight: -1.45, base_elev_m: -2200.0, province: PROVINCE_OCEANIC_ABYSS }, // South China Sea
];

/// Major Earth mountain belts and deep ocean trenches.
#[rustfmt::skip]
pub(crate) const EARTH_RIDGES: &[GeoRidgeSpec] = &[
    // Mountain belts
    GeoRidgeSpec { lat_a_deg: 34.0, lon_a_deg: 72.0, lat_b_deg: 28.0, lon_b_deg: 96.0, half_width_deg: 5.2, peak_elev_m: 5400.0, roughness_boost: 0.72, province: PROVINCE_YOUNG_OROGEN }, // Himalayas
    GeoRidgeSpec { lat_a_deg: 38.0, lon_a_deg: 74.0, lat_b_deg: 43.0, lon_b_deg: 86.0, half_width_deg: 4.0, peak_elev_m: 3100.0, roughness_boost: 0.58, province: PROVINCE_YOUNG_OROGEN }, // Pamir / Tian Shan
    GeoRidgeSpec { lat_a_deg: 9.0, lon_a_deg: -74.0, lat_b_deg: -18.0, lon_b_deg: -69.0, half_width_deg: 3.6, peak_elev_m: 4200.0, roughness_boost: 0.70, province: PROVINCE_YOUNG_OROGEN }, // Northern/Central Andes
    GeoRidgeSpec { lat_a_deg: -18.0, lon_a_deg: -69.0, lat_b_deg: -51.0, lon_b_deg: -73.0, half_width_deg: 3.2, peak_elev_m: 3600.0, roughness_boost: 0.68, province: PROVINCE_YOUNG_OROGEN }, // Southern Andes
    GeoRidgeSpec { lat_a_deg: 62.0, lon_a_deg: -140.0, lat_b_deg: 34.0, lon_b_deg: -107.0, half_width_deg: 5.5, peak_elev_m: 2600.0, roughness_boost: 0.56, province: PROVINCE_YOUNG_OROGEN }, // Rockies
    GeoRidgeSpec { lat_a_deg: 45.5, lon_a_deg: 6.5, lat_b_deg: 47.2, lon_b_deg: 15.0, half_width_deg: 2.2, peak_elev_m: 2800.0, roughness_boost: 0.62, province: PROVINCE_YOUNG_OROGEN }, // Alps
    GeoRidgeSpec { lat_a_deg: 43.3, lon_a_deg: 40.0, lat_b_deg: 41.2, lon_b_deg: 48.0, half_width_deg: 2.0, peak_elev_m: 3100.0, roughness_boost: 0.62, province: PROVINCE_YOUNG_OROGEN }, // Caucasus
    GeoRidgeSpec { lat_a_deg: 67.0, lon_a_deg: 62.0, lat_b_deg: 52.0, lon_b_deg: 58.0, half_width_deg: 2.5, peak_elev_m: 1100.0, roughness_boost: 0.32, province: PROVINCE_OLD_OROGEN_RIFT }, // Urals
    GeoRidgeSpec { lat_a_deg: 12.0, lon_a_deg: 39.0, lat_b_deg: -11.0, lon_b_deg: 34.0, half_width_deg: 3.8, peak_elev_m: 1850.0, roughness_boost: 0.44, province: PROVINCE_OLD_OROGEN_RIFT }, // East African Rift / Ethiopian Highlands
    GeoRidgeSpec { lat_a_deg: 69.0, lon_a_deg: 19.0, lat_b_deg: 60.0, lon_b_deg: 7.0, half_width_deg: 2.4, peak_elev_m: 1250.0, roughness_boost: 0.42, province: PROVINCE_OLD_OROGEN_RIFT }, // Scandinavian Mountains
    GeoRidgeSpec { lat_a_deg: 45.0, lon_a_deg: -72.0, lat_b_deg: 34.5, lon_b_deg: -84.0, half_width_deg: 2.8, peak_elev_m: 950.0, roughness_boost: 0.30, province: PROVINCE_OLD_OROGEN_RIFT }, // Appalachians

    // Deep ocean trenches
    GeoRidgeSpec { lat_a_deg: 23.0, lon_a_deg: 144.0, lat_b_deg: 12.0, lon_b_deg: 146.5, half_width_deg: 2.6, peak_elev_m: -5600.0, roughness_boost: 0.25, province: PROVINCE_SUBDUCTION_ARC }, // Mariana Trench
    GeoRidgeSpec { lat_a_deg: -16.0, lon_a_deg: -172.5, lat_b_deg: -33.0, lon_b_deg: -177.5, half_width_deg: 2.6, peak_elev_m: -4800.0, roughness_boost: 0.22, province: PROVINCE_SUBDUCTION_ARC }, // Tonga-Kermadec Trench
    GeoRidgeSpec { lat_a_deg: -8.0, lon_a_deg: -81.0, lat_b_deg: -32.0, lon_b_deg: -73.0, half_width_deg: 2.4, peak_elev_m: -3800.0, roughness_boost: 0.24, province: PROVINCE_SUBDUCTION_ARC }, // Peru-Chile Trench
    GeoRidgeSpec { lat_a_deg: -6.0, lon_a_deg: 103.0, lat_b_deg: -11.5, lon_b_deg: 118.0, half_width_deg: 2.4, peak_elev_m: -3600.0, roughness_boost: 0.22, province: PROVINCE_SUBDUCTION_ARC }, // Java Trench
];
