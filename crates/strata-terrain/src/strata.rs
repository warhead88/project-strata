use crate::earth_data::{
    PROVINCE_CONTINENTAL_SHELF, PROVINCE_CRATON, PROVINCE_OCEANIC_ABYSS, PROVINCE_OLD_OROGEN_RIFT,
    PROVINCE_POLAR_SHIELD, PROVINCE_SEDIMENTARY_BASIN, PROVINCE_SUBDUCTION_ARC,
    PROVINCE_YOUNG_OROGEN,
};

/// Maximum number of contiguous bedrock strata in a [`StrataColumn`].
pub const MAX_STRATA_LAYERS: usize = 5;

/// Deepest depth in meters covered by a [`StrataColumn`].
pub const MAX_STRATA_DEPTH_M: f32 = 10_000.0;

/// Macro tectonic-geological province mapped 1-to-1 from macro province IDs `0..=7`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum GeologyProvince {
    /// Deep oceanic basaltic crust and abyssal plains (`0`).
    OceanicAbyss,
    /// Shallow continental shelf and passive margin (`1`).
    ContinentalShelf,
    /// Ancient stable Precambrian craton / shield (`2`).
    CratonShield,
    /// Subsiding sedimentary basin and lowland plains (`3`).
    SedimentaryBasin,
    /// Young Cenozoic/Mesozoic collisional mountain belt (`4`).
    YoungOrogen,
    /// Ancient Paleozoic mountain belt or continental rift (`5`).
    OldOrogenRift,
    /// Active subduction trench or volcanic island arc (`6`).
    SubductionArc,
    /// Polar ice-capped continental shield (`7`).
    PolarShield,
}

impl GeologyProvince {
    /// Maps a macro geology province ID (`0..=7`) to [`GeologyProvince`].
    #[must_use]
    pub const fn from_id(id: u8) -> Self {
        match id {
            PROVINCE_CONTINENTAL_SHELF => Self::ContinentalShelf,
            PROVINCE_CRATON => Self::CratonShield,
            PROVINCE_SEDIMENTARY_BASIN => Self::SedimentaryBasin,
            PROVINCE_YOUNG_OROGEN => Self::YoungOrogen,
            PROVINCE_OLD_OROGEN_RIFT => Self::OldOrogenRift,
            PROVINCE_SUBDUCTION_ARC => Self::SubductionArc,
            PROVINCE_POLAR_SHIELD => Self::PolarShield,
            _ => Self::OceanicAbyss,
        }
    }

    /// Returns the numeric macro geology province identifier (`0..=7`).
    #[must_use]
    pub const fn id(self) -> u8 {
        match self {
            Self::OceanicAbyss => PROVINCE_OCEANIC_ABYSS,
            Self::ContinentalShelf => PROVINCE_CONTINENTAL_SHELF,
            Self::CratonShield => PROVINCE_CRATON,
            Self::SedimentaryBasin => PROVINCE_SEDIMENTARY_BASIN,
            Self::YoungOrogen => PROVINCE_YOUNG_OROGEN,
            Self::OldOrogenRift => PROVINCE_OLD_OROGEN_RIFT,
            Self::SubductionArc => PROVINCE_SUBDUCTION_ARC,
            Self::PolarShield => PROVINCE_POLAR_SHIELD,
        }
    }

    /// Human-readable geological province label.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::OceanicAbyss => "Oceanic Abyss",
            Self::ContinentalShelf => "Continental Shelf",
            Self::CratonShield => "Craton Shield",
            Self::SedimentaryBasin => "Sedimentary Basin",
            Self::YoungOrogen => "Young Orogen",
            Self::OldOrogenRift => "Old Orogen / Rift",
            Self::SubductionArc => "Subduction Arc",
            Self::PolarShield => "Polar Shield",
        }
    }
}

/// Unconsolidated surface regolith, soil, ice, or exposed rock classification.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SurfaceMaterial {
    /// Fertile temperate or tropical loam soil.
    LoamSoil,
    /// Riverine and deltaic alluvial silt.
    AlluvialSilt,
    /// Aeolian desert or coastal sand.
    Sand,
    /// Waterlogged organic peat wetland.
    PeatWetland,
    /// Unsorted glacial till and moraine gravel.
    GlacialTill,
    /// Permanently frozen active-layer regolith.
    Permafrost,
    /// Bare steep bedrock or alpine scree.
    ExposedRock,
    /// Permanent continental or polar ice sheet.
    IceSheet,
    /// Submerged marine ooze and shelf sediment.
    MarineSediment,
}

/// Dominant lithology of an underground bedrock stratum.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BedrockKind {
    /// Crystalline felsic plutonic basement.
    Granite,
    /// Dense mafic volcanic or oceanic crust.
    Basalt,
    /// Clastic porous sedimentary rock.
    Sandstone,
    /// Carbonate rock prone to karst dissolution.
    LimestoneKarst,
    /// Fine-grained fissile mudrock.
    Shale,
    /// Foliated or high-grade metamorphic rock.
    Metamorphic,
    /// Evaporitic halite/gypsum/potash sequence.
    Evaporite,
}

/// Normalized `[0.0, 1.0]` deterministic mineral and energy exploration potentials.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MineralPotential {
    /// Iron ore potential.
    pub iron: f32,
    /// Coal seam potential.
    pub coal: f32,
    /// Oil and natural gas potential.
    pub hydrocarbons: f32,
    /// Copper and porphyry sulfide potential.
    pub copper: f32,
    /// Lithium brine or pegmatite potential.
    pub lithium: f32,
    /// Rare-earth element potential.
    pub rare_earths: f32,
    /// Uranium deposit potential.
    pub uranium: f32,
    /// Industrial limestone / cement feedstock potential.
    pub limestone: f32,
    /// Deep geothermal energy potential.
    pub geothermal: f32,
}

/// Single contiguous depth interval inside a [`StrataColumn`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct StratumLayer {
    /// Top depth below the surface in meters (`>= 0.0`).
    pub top_depth_m: f32,
    /// Bottom depth below the surface in meters (`<= 10_000.0`).
    pub bottom_depth_m: f32,
    /// Dominant bedrock lithology in this interval.
    pub bedrock: BedrockKind,
    /// Mechanical rock hardness in `[0.0, 1.0]`.
    pub hardness: f32,
    /// Tunnel and cavern structural stability in `[0.0, 1.0]`.
    pub structural_stability: f32,
    /// Hydraulic permeability in `[0.0, 1.0]`.
    pub permeability: f32,
    /// Geothermal gradient across this stratum in `deg C / km`.
    pub thermal_gradient_c_per_km: f32,
}

/// Stack-allocated vertical geological profile covering `0.0 ..= 10_000.0 m` below the surface.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct StrataColumn {
    /// Regional tectonic-geological province.
    pub province: GeologyProvince,
    /// Surface regolith or ice cover material.
    pub surface_material: SurfaceMaterial,
    /// Thickness of unconsolidated overburden above bedrock in meters.
    pub overburden_thickness_m: f32,
    /// Depth of permanently frozen ground in meters (`0.0` when unfrozen).
    pub permafrost_depth_m: f32,
    /// Depth to groundwater table in meters (`0.0` when saturated or submerged).
    pub water_table_depth_m: f32,
    /// Deterministic normalized mineral and geothermal potentials.
    pub mineral_potential: MineralPotential,
    layers: [StratumLayer; MAX_STRATA_LAYERS],
    layer_count: u8,
}

impl StrataColumn {
    #[must_use]
    pub(crate) fn new(
        province: GeologyProvince,
        surface_material: SurfaceMaterial,
        depths_m: [f32; 3],
        mineral_potential: MineralPotential,
        layers: [StratumLayer; MAX_STRATA_LAYERS],
        layer_count: u8,
    ) -> Self {
        Self {
            province,
            surface_material,
            overburden_thickness_m: depths_m[0],
            permafrost_depth_m: depths_m[1],
            water_table_depth_m: depths_m[2],
            mineral_potential,
            layers,
            layer_count: layer_count.clamp(1, u8::try_from(MAX_STRATA_LAYERS).unwrap_or(5)),
        }
    }

    /// Contiguous strata layers ordered from surface (`0.0 m`) down to `10_000.0 m`.
    #[must_use]
    pub fn layers(&self) -> &[StratumLayer] {
        let count = usize::from(self.layer_count).clamp(1, MAX_STRATA_LAYERS);
        &self.layers[..count]
    }

    /// Returns the [`StratumLayer`] containing `depth_m` (clamped to `0.0 ..= 10_000.0 m`).
    #[must_use]
    pub fn layer_at_depth(&self, depth_m: f64) -> &StratumLayer {
        let d = if depth_m.is_finite() {
            depth_m.clamp(0.0, f64::from(MAX_STRATA_DEPTH_M))
        } else {
            0.0
        };
        let active = self.layers();
        let mut chosen = &active[0];
        for layer in active {
            chosen = layer;
            if d <= f64::from(layer.bottom_depth_m) {
                break;
            }
        }
        chosen
    }

    /// Dimensionless excavation cost multiplier (`>= 0.7`) at `depth_m`.
    #[must_use]
    #[expect(
        clippy::cast_possible_truncation,
        reason = "cost multiplier intentionally returns compact f32"
    )]
    pub fn excavation_cost_multiplier(&self, depth_m: f64) -> f32 {
        let d = if depth_m.is_finite() {
            depth_m.max(0.0)
        } else {
            0.0
        };
        let layer = self.layer_at_depth(d);
        let in_overburden = d <= f64::from(self.overburden_thickness_m);
        let base_rock = if in_overburden {
            0.78 + f64::from(layer.hardness) * 0.35
        } else {
            1.05 + f64::from(layer.hardness) * 2.15
                + (1.0 - f64::from(layer.structural_stability)) * 0.95
        };
        let frost_penalty =
            if self.permafrost_depth_m > 0.0 && d <= f64::from(self.permafrost_depth_m) {
                0.55
            } else {
                0.0
            };
        let water_penalty = if d > f64::from(self.water_table_depth_m) && layer.permeability > 0.25
        {
            f64::from(layer.permeability) * 0.65
        } else {
            0.0
        };
        let depth_factor = 1.0 + (d / 3_500.0).min(2.5);
        ((base_rock + frost_penalty + water_penalty) * depth_factor) as f32
    }

    /// Normalized `[0.0, 1.0]` suitability score for subterranean construction at `depth_m`.
    #[must_use]
    #[expect(
        clippy::cast_possible_truncation,
        reason = "suitability score intentionally returns normalized f32"
    )]
    pub fn underground_suitability(&self, depth_m: f64) -> f32 {
        let d = if depth_m.is_finite() {
            depth_m.max(0.0)
        } else {
            0.0
        };
        if self.province == GeologyProvince::OceanicAbyss {
            return 0.05;
        }
        let layer = self.layer_at_depth(d);
        let stability = f64::from(layer.structural_stability);
        let dry_seal = 1.0 - f64::from(layer.permeability);
        let lith_bonus = match layer.bedrock {
            BedrockKind::Granite | BedrockKind::Metamorphic => 0.14,
            BedrockKind::Basalt | BedrockKind::Sandstone => 0.04,
            BedrockKind::LimestoneKarst => -0.10,
            BedrockKind::Shale | BedrockKind::Evaporite => -0.14,
        };
        let cover_bonus = if d >= f64::from(self.overburden_thickness_m).max(12.0) {
            0.08
        } else {
            -0.12
        };
        let water_penalty = if d > f64::from(self.water_table_depth_m) {
            f64::from(layer.permeability) * 0.28
        } else {
            0.0
        };
        let heat_penalty =
            ((d / 1_000.0) * f64::from(layer.thermal_gradient_c_per_km) - 45.0).max(0.0) / 120.0;
        let score = 0.52 * stability + 0.34 * dry_seal + lith_bonus + cover_bonus
            - water_penalty
            - heat_penalty;
        score.clamp(0.0, 1.0) as f32
    }

    /// Ambient bedrock temperature in degrees Celsius at `depth_m` given `surface_temp_c`.
    #[must_use]
    #[expect(
        clippy::cast_possible_truncation,
        reason = "rock temperature intentionally returns f32"
    )]
    pub fn ambient_rock_temp_c(&self, surface_temp_c: f32, depth_m: f64) -> f32 {
        let d = if depth_m.is_finite() {
            depth_m.max(0.0)
        } else {
            0.0
        };
        let mut temp = f64::from(surface_temp_c);
        let mut covered = 0.0_f64;
        for layer in self.layers() {
            let top = f64::from(layer.top_depth_m);
            let bot = f64::from(layer.bottom_depth_m);
            if d <= top {
                break;
            }
            let span = d.min(bot) - top;
            if span > 0.0 {
                temp += (span / 1_000.0) * f64::from(layer.thermal_gradient_c_per_km);
                covered = d.min(bot);
            }
        }
        if d > covered
            && let Some(deepest) = self.layers().last()
        {
            temp += ((d - covered) / 1_000.0) * f64::from(deepest.thermal_gradient_c_per_km);
        }
        temp as f32
    }
}
