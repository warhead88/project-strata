use crate::hydrology::{Navigability, Salinity};

pub(crate) struct EarthLakeSpec {
    pub name: &'static str,
    pub lat_deg: f64,
    pub lon_deg: f64,
    pub radius_deg: f64,
    pub surface_elevation_m: f64,
    pub max_depth_m: f64,
    pub salinity: Salinity,
}

pub(crate) struct EarthRiverWaypointSpec {
    pub lat_deg: f64,
    pub lon_deg: f64,
    pub surface_elevation_m: f64,
    pub half_width_m: f64,
    pub channel_depth_m: f64,
    pub valley_half_width_m: f64,
    pub discharge_m3s: f64,
}

pub(crate) struct EarthRiverSpec {
    pub name: &'static str,
    pub navigability: Navigability,
    pub waypoints: &'static [EarthRiverWaypointSpec],
}

#[rustfmt::skip]
pub(crate) const EARTH_LAKES: &[EarthLakeSpec] = &[
    EarthLakeSpec { name: "Caspian Sea", lat_deg: 41.5, lon_deg: 50.5, radius_deg: 2.35, surface_elevation_m: -28.0, max_depth_m: 1025.0, salinity: Salinity::Saline },
    EarthLakeSpec { name: "Lake Baikal", lat_deg: 53.5, lon_deg: 108.2, radius_deg: 1.15, surface_elevation_m: 455.0, max_depth_m: 1642.0, salinity: Salinity::Freshwater },
    EarthLakeSpec { name: "Lake Victoria", lat_deg: -1.0, lon_deg: 33.0, radius_deg: 1.25, surface_elevation_m: 1134.0, max_depth_m: 84.0, salinity: Salinity::Freshwater },
    EarthLakeSpec { name: "Great Lakes (Superior)", lat_deg: 47.7, lon_deg: -87.5, radius_deg: 1.45, surface_elevation_m: 183.0, max_depth_m: 406.0, salinity: Salinity::Freshwater },
    EarthLakeSpec { name: "Great Lakes (Huron-Michigan)", lat_deg: 44.8, lon_deg: -84.2, radius_deg: 1.35, surface_elevation_m: 177.0, max_depth_m: 281.0, salinity: Salinity::Freshwater },
    EarthLakeSpec { name: "Lake Tanganyika", lat_deg: -6.2, lon_deg: 29.6, radius_deg: 0.95, surface_elevation_m: 773.0, max_depth_m: 1470.0, salinity: Salinity::Freshwater },
    EarthLakeSpec { name: "Lake Titicaca", lat_deg: -15.8, lon_deg: -69.4, radius_deg: 0.60, surface_elevation_m: 3812.0, max_depth_m: 281.0, salinity: Salinity::Freshwater },
];

#[rustfmt::skip]
const AMAZON_WPS: &[EarthRiverWaypointSpec] = &[
    EarthRiverWaypointSpec { lat_deg: -10.5, lon_deg: -75.5, surface_elevation_m: 2800.0, half_width_m: 400.0, channel_depth_m: 6.0, valley_half_width_m: 14_000.0, discharge_m3s: 3_000.0 },
    EarthRiverWaypointSpec { lat_deg: -6.5, lon_deg: -74.5, surface_elevation_m: 180.0, half_width_m: 1_800.0, channel_depth_m: 14.0, valley_half_width_m: 28_000.0, discharge_m3s: 32_000.0 },
    EarthRiverWaypointSpec { lat_deg: -3.7, lon_deg: -70.0, surface_elevation_m: 85.0, half_width_m: 3_500.0, channel_depth_m: 22.0, valley_half_width_m: 40_000.0, discharge_m3s: 80_000.0 },
    EarthRiverWaypointSpec { lat_deg: -3.1, lon_deg: -60.0, surface_elevation_m: 28.0, half_width_m: 5_500.0, channel_depth_m: 35.0, valley_half_width_m: 55_000.0, discharge_m3s: 155_000.0 },
    EarthRiverWaypointSpec { lat_deg: -1.8, lon_deg: -54.0, surface_elevation_m: 10.0, half_width_m: 7_500.0, channel_depth_m: 42.0, valley_half_width_m: 65_000.0, discharge_m3s: 195_000.0 },
    EarthRiverWaypointSpec { lat_deg: -0.2, lon_deg: -49.5, surface_elevation_m: 0.0, half_width_m: 12_000.0, channel_depth_m: 48.0, valley_half_width_m: 80_000.0, discharge_m3s: 209_000.0 },
];

#[rustfmt::skip]
const NILE_WPS: &[EarthRiverWaypointSpec] = &[
    EarthRiverWaypointSpec { lat_deg: 0.5, lon_deg: 33.2, surface_elevation_m: 1134.0, half_width_m: 600.0, channel_depth_m: 7.0, valley_half_width_m: 15_000.0, discharge_m3s: 900.0 },
    EarthRiverWaypointSpec { lat_deg: 8.5, lon_deg: 31.0, surface_elevation_m: 420.0, half_width_m: 1_200.0, channel_depth_m: 9.0, valley_half_width_m: 24_000.0, discharge_m3s: 1_500.0 },
    EarthRiverWaypointSpec { lat_deg: 15.6, lon_deg: 32.5, surface_elevation_m: 375.0, half_width_m: 1_500.0, channel_depth_m: 10.0, valley_half_width_m: 25_000.0, discharge_m3s: 2_800.0 },
    EarthRiverWaypointSpec { lat_deg: 24.0, lon_deg: 32.9, surface_elevation_m: 95.0, half_width_m: 1_800.0, channel_depth_m: 11.0, valley_half_width_m: 28_000.0, discharge_m3s: 2_700.0 },
    EarthRiverWaypointSpec { lat_deg: 31.2, lon_deg: 30.8, surface_elevation_m: 0.0, half_width_m: 2_600.0, channel_depth_m: 14.0, valley_half_width_m: 35_000.0, discharge_m3s: 2_830.0 },
];

#[rustfmt::skip]
const MISSISSIPPI_WPS: &[EarthRiverWaypointSpec] = &[
    EarthRiverWaypointSpec { lat_deg: 47.2, lon_deg: -95.2, surface_elevation_m: 450.0, half_width_m: 400.0, channel_depth_m: 5.0, valley_half_width_m: 14_000.0, discharge_m3s: 400.0 },
    EarthRiverWaypointSpec { lat_deg: 41.5, lon_deg: -90.5, surface_elevation_m: 170.0, half_width_m: 1_200.0, channel_depth_m: 9.0, valley_half_width_m: 25_000.0, discharge_m3s: 4_500.0 },
    EarthRiverWaypointSpec { lat_deg: 37.0, lon_deg: -89.2, surface_elevation_m: 85.0, half_width_m: 2_000.0, channel_depth_m: 14.0, valley_half_width_m: 36_000.0, discharge_m3s: 12_000.0 },
    EarthRiverWaypointSpec { lat_deg: 32.3, lon_deg: -90.9, surface_elevation_m: 24.0, half_width_m: 2_600.0, channel_depth_m: 18.0, valley_half_width_m: 45_000.0, discharge_m3s: 16_000.0 },
    EarthRiverWaypointSpec { lat_deg: 29.1, lon_deg: -89.3, surface_elevation_m: 0.0, half_width_m: 3_800.0, channel_depth_m: 24.0, valley_half_width_m: 55_000.0, discharge_m3s: 18_400.0 },
];

#[rustfmt::skip]
const YANGTZE_WPS: &[EarthRiverWaypointSpec] = &[
    EarthRiverWaypointSpec { lat_deg: 33.4, lon_deg: 91.5, surface_elevation_m: 4600.0, half_width_m: 450.0, channel_depth_m: 6.0, valley_half_width_m: 16_000.0, discharge_m3s: 1_200.0 },
    EarthRiverWaypointSpec { lat_deg: 28.8, lon_deg: 104.6, surface_elevation_m: 260.0, half_width_m: 1_400.0, channel_depth_m: 12.0, valley_half_width_m: 28_000.0, discharge_m3s: 9_500.0 },
    EarthRiverWaypointSpec { lat_deg: 30.6, lon_deg: 114.3, surface_elevation_m: 22.0, half_width_m: 2_800.0, channel_depth_m: 18.0, valley_half_width_m: 42_000.0, discharge_m3s: 23_000.0 },
    EarthRiverWaypointSpec { lat_deg: 31.5, lon_deg: 121.6, surface_elevation_m: 0.0, half_width_m: 6_000.0, channel_depth_m: 25.0, valley_half_width_m: 60_000.0, discharge_m3s: 30_100.0 },
];

#[rustfmt::skip]
const DANUBE_WPS: &[EarthRiverWaypointSpec] = &[
    EarthRiverWaypointSpec { lat_deg: 48.1, lon_deg: 8.2, surface_elevation_m: 670.0, half_width_m: 350.0, channel_depth_m: 5.0, valley_half_width_m: 12_000.0, discharge_m3s: 400.0 },
    EarthRiverWaypointSpec { lat_deg: 48.2, lon_deg: 16.4, surface_elevation_m: 155.0, half_width_m: 900.0, channel_depth_m: 8.0, valley_half_width_m: 20_000.0, discharge_m3s: 2_000.0 },
    EarthRiverWaypointSpec { lat_deg: 44.8, lon_deg: 20.4, surface_elevation_m: 72.0, half_width_m: 1_400.0, channel_depth_m: 11.0, valley_half_width_m: 28_000.0, discharge_m3s: 5_200.0 },
    EarthRiverWaypointSpec { lat_deg: 45.2, lon_deg: 29.6, surface_elevation_m: 0.0, half_width_m: 2_400.0, channel_depth_m: 15.0, valley_half_width_m: 38_000.0, discharge_m3s: 6_500.0 },
];

#[rustfmt::skip]
const VOLGA_WPS: &[EarthRiverWaypointSpec] = &[
    EarthRiverWaypointSpec { lat_deg: 57.2, lon_deg: 32.5, surface_elevation_m: 228.0, half_width_m: 400.0, channel_depth_m: 5.0, valley_half_width_m: 14_000.0, discharge_m3s: 500.0 },
    EarthRiverWaypointSpec { lat_deg: 56.3, lon_deg: 44.0, surface_elevation_m: 64.0, half_width_m: 1_400.0, channel_depth_m: 10.0, valley_half_width_m: 26_000.0, discharge_m3s: 3_200.0 },
    EarthRiverWaypointSpec { lat_deg: 48.7, lon_deg: 44.5, surface_elevation_m: -12.0, half_width_m: 2_200.0, channel_depth_m: 14.0, valley_half_width_m: 34_000.0, discharge_m3s: 7_800.0 },
    EarthRiverWaypointSpec { lat_deg: 45.8, lon_deg: 47.9, surface_elevation_m: -28.0, half_width_m: 3_000.0, channel_depth_m: 16.0, valley_half_width_m: 42_000.0, discharge_m3s: 8_060.0 },
];

#[rustfmt::skip]
const RHINE_WPS: &[EarthRiverWaypointSpec] = &[
    EarthRiverWaypointSpec { lat_deg: 46.6, lon_deg: 8.7, surface_elevation_m: 1600.0, half_width_m: 300.0, channel_depth_m: 5.0, valley_half_width_m: 10_000.0, discharge_m3s: 300.0 },
    EarthRiverWaypointSpec { lat_deg: 49.5, lon_deg: 8.4, surface_elevation_m: 90.0, half_width_m: 750.0, channel_depth_m: 8.0, valley_half_width_m: 18_000.0, discharge_m3s: 1_400.0 },
    EarthRiverWaypointSpec { lat_deg: 51.9, lon_deg: 4.1, surface_elevation_m: 0.0, half_width_m: 1_600.0, channel_depth_m: 15.0, valley_half_width_m: 26_000.0, discharge_m3s: 2_900.0 },
];

#[rustfmt::skip]
const CONGO_WPS: &[EarthRiverWaypointSpec] = &[
    EarthRiverWaypointSpec { lat_deg: -9.2, lon_deg: 26.0, surface_elevation_m: 920.0, half_width_m: 600.0, channel_depth_m: 7.0, valley_half_width_m: 16_000.0, discharge_m3s: 2_500.0 },
    EarthRiverWaypointSpec { lat_deg: 0.5, lon_deg: 25.2, surface_elevation_m: 390.0, half_width_m: 2_200.0, channel_depth_m: 14.0, valley_half_width_m: 32_000.0, discharge_m3s: 18_000.0 },
    EarthRiverWaypointSpec { lat_deg: -4.3, lon_deg: 15.3, surface_elevation_m: 270.0, half_width_m: 4_000.0, channel_depth_m: 22.0, valley_half_width_m: 46_000.0, discharge_m3s: 38_000.0 },
    EarthRiverWaypointSpec { lat_deg: -6.0, lon_deg: 12.4, surface_elevation_m: 0.0, half_width_m: 5_200.0, channel_depth_m: 30.0, valley_half_width_m: 55_000.0, discharge_m3s: 41_200.0 },
];

#[rustfmt::skip]
const GANGES_WPS: &[EarthRiverWaypointSpec] = &[
    EarthRiverWaypointSpec { lat_deg: 30.9, lon_deg: 78.9, surface_elevation_m: 3100.0, half_width_m: 450.0, channel_depth_m: 6.0, valley_half_width_m: 16_000.0, discharge_m3s: 900.0 },
    EarthRiverWaypointSpec { lat_deg: 25.3, lon_deg: 83.0, surface_elevation_m: 75.0, half_width_m: 2_000.0, channel_depth_m: 12.0, valley_half_width_m: 32_000.0, discharge_m3s: 11_000.0 },
    EarthRiverWaypointSpec { lat_deg: 22.2, lon_deg: 90.3, surface_elevation_m: 0.0, half_width_m: 5_500.0, channel_depth_m: 22.0, valley_half_width_m: 58_000.0, discharge_m3s: 38_100.0 },
];

#[rustfmt::skip]
const MEKONG_WPS: &[EarthRiverWaypointSpec] = &[
    EarthRiverWaypointSpec { lat_deg: 33.2, lon_deg: 94.0, surface_elevation_m: 4500.0, half_width_m: 400.0, channel_depth_m: 6.0, valley_half_width_m: 15_000.0, discharge_m3s: 700.0 },
    EarthRiverWaypointSpec { lat_deg: 19.9, lon_deg: 102.1, surface_elevation_m: 290.0, half_width_m: 1_200.0, channel_depth_m: 11.0, valley_half_width_m: 26_000.0, discharge_m3s: 4_500.0 },
    EarthRiverWaypointSpec { lat_deg: 11.6, lon_deg: 104.9, surface_elevation_m: 10.0, half_width_m: 2_600.0, channel_depth_m: 16.0, valley_half_width_m: 40_000.0, discharge_m3s: 13_500.0 },
    EarthRiverWaypointSpec { lat_deg: 9.8, lon_deg: 106.6, surface_elevation_m: 0.0, half_width_m: 4_200.0, channel_depth_m: 20.0, valley_half_width_m: 50_000.0, discharge_m3s: 16_000.0 },
];

#[rustfmt::skip]
const OB_WPS: &[EarthRiverWaypointSpec] = &[
    EarthRiverWaypointSpec { lat_deg: 52.4, lon_deg: 85.0, surface_elevation_m: 220.0, half_width_m: 700.0, channel_depth_m: 7.0, valley_half_width_m: 20_000.0, discharge_m3s: 1_600.0 },
    EarthRiverWaypointSpec { lat_deg: 61.1, lon_deg: 69.0, surface_elevation_m: 28.0, half_width_m: 2_400.0, channel_depth_m: 14.0, valley_half_width_m: 38_000.0, discharge_m3s: 10_000.0 },
    EarthRiverWaypointSpec { lat_deg: 66.8, lon_deg: 69.5, surface_elevation_m: 0.0, half_width_m: 4_500.0, channel_depth_m: 18.0, valley_half_width_m: 52_000.0, discharge_m3s: 12_475.0 },
];

#[rustfmt::skip]
const YENISEI_WPS: &[EarthRiverWaypointSpec] = &[
    EarthRiverWaypointSpec { lat_deg: 51.7, lon_deg: 94.5, surface_elevation_m: 620.0, half_width_m: 600.0, channel_depth_m: 7.0, valley_half_width_m: 18_000.0, discharge_m3s: 1_500.0 },
    EarthRiverWaypointSpec { lat_deg: 60.2, lon_deg: 90.2, surface_elevation_m: 68.0, half_width_m: 2_200.0, channel_depth_m: 14.0, valley_half_width_m: 34_000.0, discharge_m3s: 11_000.0 },
    EarthRiverWaypointSpec { lat_deg: 71.8, lon_deg: 83.0, surface_elevation_m: 0.0, half_width_m: 4_800.0, channel_depth_m: 20.0, valley_half_width_m: 55_000.0, discharge_m3s: 19_600.0 },
];

#[rustfmt::skip]
const LENA_WPS: &[EarthRiverWaypointSpec] = &[
    EarthRiverWaypointSpec { lat_deg: 54.0, lon_deg: 108.0, surface_elevation_m: 950.0, half_width_m: 550.0, channel_depth_m: 7.0, valley_half_width_m: 18_000.0, discharge_m3s: 1_100.0 },
    EarthRiverWaypointSpec { lat_deg: 62.0, lon_deg: 129.7, surface_elevation_m: 92.0, half_width_m: 2_800.0, channel_depth_m: 14.0, valley_half_width_m: 38_000.0, discharge_m3s: 11_500.0 },
    EarthRiverWaypointSpec { lat_deg: 72.5, lon_deg: 127.0, surface_elevation_m: 0.0, half_width_m: 5_000.0, channel_depth_m: 20.0, valley_half_width_m: 55_000.0, discharge_m3s: 16_870.0 },
];

#[rustfmt::skip]
const PARANA_WPS: &[EarthRiverWaypointSpec] = &[
    EarthRiverWaypointSpec { lat_deg: -20.1, lon_deg: -51.0, surface_elevation_m: 340.0, half_width_m: 1_200.0, channel_depth_m: 10.0, valley_half_width_m: 24_000.0, discharge_m3s: 6_000.0 },
    EarthRiverWaypointSpec { lat_deg: -27.5, lon_deg: -58.8, surface_elevation_m: 52.0, half_width_m: 2_600.0, channel_depth_m: 15.0, valley_half_width_m: 36_000.0, discharge_m3s: 15_000.0 },
    EarthRiverWaypointSpec { lat_deg: -34.2, lon_deg: -58.4, surface_elevation_m: 0.0, half_width_m: 4_500.0, channel_depth_m: 20.0, valley_half_width_m: 50_000.0, discharge_m3s: 17_300.0 },
];

#[rustfmt::skip]
const MURRAY_WPS: &[EarthRiverWaypointSpec] = &[
    EarthRiverWaypointSpec { lat_deg: -36.5, lon_deg: 148.0, surface_elevation_m: 900.0, half_width_m: 300.0, channel_depth_m: 5.0, valley_half_width_m: 12_000.0, discharge_m3s: 220.0 },
    EarthRiverWaypointSpec { lat_deg: -34.2, lon_deg: 142.1, surface_elevation_m: 38.0, half_width_m: 650.0, channel_depth_m: 7.0, valley_half_width_m: 18_000.0, discharge_m3s: 520.0 },
    EarthRiverWaypointSpec { lat_deg: -35.5, lon_deg: 138.9, surface_elevation_m: 0.0, half_width_m: 1_100.0, channel_depth_m: 10.0, valley_half_width_m: 24_000.0, discharge_m3s: 767.0 },
];

pub(crate) const EARTH_RIVERS: &[EarthRiverSpec] = &[
    EarthRiverSpec {
        name: "Amazon",
        navigability: Navigability::SeaGoingEstuary,
        waypoints: AMAZON_WPS,
    },
    EarthRiverSpec {
        name: "Nile",
        navigability: Navigability::RiverVessel,
        waypoints: NILE_WPS,
    },
    EarthRiverSpec {
        name: "Mississippi",
        navigability: Navigability::SeaGoingEstuary,
        waypoints: MISSISSIPPI_WPS,
    },
    EarthRiverSpec {
        name: "Yangtze",
        navigability: Navigability::SeaGoingEstuary,
        waypoints: YANGTZE_WPS,
    },
    EarthRiverSpec {
        name: "Danube",
        navigability: Navigability::RiverVessel,
        waypoints: DANUBE_WPS,
    },
    EarthRiverSpec {
        name: "Volga",
        navigability: Navigability::RiverVessel,
        waypoints: VOLGA_WPS,
    },
    EarthRiverSpec {
        name: "Rhine",
        navigability: Navigability::RiverVessel,
        waypoints: RHINE_WPS,
    },
    EarthRiverSpec {
        name: "Congo",
        navigability: Navigability::Barge,
        waypoints: CONGO_WPS,
    },
    EarthRiverSpec {
        name: "Ganges",
        navigability: Navigability::RiverVessel,
        waypoints: GANGES_WPS,
    },
    EarthRiverSpec {
        name: "Mekong",
        navigability: Navigability::RiverVessel,
        waypoints: MEKONG_WPS,
    },
    EarthRiverSpec {
        name: "Ob",
        navigability: Navigability::RiverVessel,
        waypoints: OB_WPS,
    },
    EarthRiverSpec {
        name: "Yenisei",
        navigability: Navigability::SeaGoingEstuary,
        waypoints: YENISEI_WPS,
    },
    EarthRiverSpec {
        name: "Lena",
        navigability: Navigability::RiverVessel,
        waypoints: LENA_WPS,
    },
    EarthRiverSpec {
        name: "Parana",
        navigability: Navigability::SeaGoingEstuary,
        waypoints: PARANA_WPS,
    },
    EarthRiverSpec {
        name: "Murray",
        navigability: Navigability::Barge,
        waypoints: MURRAY_WPS,
    },
];
