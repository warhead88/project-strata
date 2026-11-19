use std::collections::{HashMap, HashSet};

use bevy::asset::RenderAssetUsages;
use bevy::mesh::{Indices, PrimitiveTopology};
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use strata_globe::{
    CoarserEdges, EcefMeters, LocalOrigin, LodConfig, PlanetSpec, TileId, TileRenderMesh,
    build_tile_render_mesh, select_tiles,
};

use crate::camera::{GlobeCameraState, OriginState};
use crate::snapshot::ClientWorldView;

/// Cached GPU tile mesh and metadata for a single visible [`TileId`].
struct CachedTile {
    entity: Entity,
    mesh: Handle<Mesh>,
    coarser_edges: CoarserEdges,
    baked_origin: LocalOrigin,
}

/// Client-side tile mesh cache, LOD hysteresis set, and cube-face materials.
#[derive(Resource)]
pub struct GlobeRenderState {
    tiles: HashMap<TileId, CachedTile>,
    previously_split: HashSet<TileId>,
    baked_planet: Option<PlanetSpec>,
    face_materials: [Handle<StandardMaterial>; 6],
    /// Current LOD configuration used by the client renderer.
    pub lod_config: LodConfig,
    /// Number of tiles active on screen this frame.
    pub active_tile_count: usize,
    /// Minimum LOD among active tiles.
    pub min_lod: u8,
    /// Maximum LOD among active tiles.
    pub max_lod: u8,
}

impl GlobeRenderState {
    /// Initializes face materials with a procedural tile-grid texture so tile
    /// boundaries, `L/L+1` stitching, and cube-face seams are visually inspectable.
    fn initialize(images: &mut Assets<Image>, materials: &mut Assets<StandardMaterial>) -> Self {
        let grid_texture = images.add(build_tile_grid_image());
        let face_colors = [
            Color::srgb(0.28, 0.48, 0.62), // +X
            Color::srgb(0.32, 0.56, 0.50), // -X
            Color::srgb(0.58, 0.62, 0.68), // +Y (polar)
            Color::srgb(0.54, 0.58, 0.64), // -Y (polar)
            Color::srgb(0.34, 0.52, 0.42), // +Z
            Color::srgb(0.46, 0.48, 0.36), // -Z
        ];
        let face_materials = face_colors.map(|color| {
            materials.add(StandardMaterial {
                base_color: color,
                base_color_texture: Some(grid_texture.clone()),
                perceptual_roughness: 0.88,
                reflectance: 0.15,
                ..default()
            })
        });

        Self {
            tiles: HashMap::new(),
            previously_split: HashSet::new(),
            baked_planet: None,
            face_materials,
            lod_config: LodConfig::standard(),
            active_tile_count: 0,
            min_lod: 0,
            max_lod: 0,
        }
    }
}

/// Startup system initializing tile materials and render cache.
pub fn setup_globe_materials(
    mut commands: Commands,
    mut images: ResMut<Assets<Image>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    let state = GlobeRenderState::initialize(&mut images, &mut materials);
    commands.insert_resource(state);
}

/// Synchronizes visible Bevy tile meshes with [`strata_globe::select_tiles`] and [`OriginState`].
#[expect(
    clippy::needless_pass_by_value,
    reason = "Bevy ECS system parameters require Res<T> by value"
)]
pub fn sync_globe_tiles(
    mut commands: Commands,
    windows: Query<&Window>,
    world_view: Res<ClientWorldView>,
    cam_state: Res<GlobeCameraState>,
    origin_state: Res<OriginState>,
    mut render_state: ResMut<GlobeRenderState>,
    mut meshes: ResMut<Assets<Mesh>>,
) {
    let (width_px, height_px) = windows.iter().next().map_or((1280.0_f64, 720.0_f64), |w| {
        (
            f64::from(w.width()).max(1.0),
            f64::from(w.height()).max(1.0),
        )
    });
    let aspect = width_px / height_px;
    let Ok(cam_view) = cam_state.to_camera_view_f64(aspect, height_px) else {
        return;
    };

    let planet = world_view.snapshot().planet;
    if render_state.baked_planet != Some(planet) {
        for (_, removed) in render_state.tiles.drain() {
            commands.entity(removed.entity).despawn();
            meshes.remove(&removed.mesh);
        }
        render_state.previously_split.clear();
        render_state.baked_planet = Some(planet);
    }

    let selection = select_tiles(
        planet,
        &cam_view,
        render_state.lod_config,
        &mut render_state.previously_split,
    );

    render_state.active_tile_count = selection.tiles.len();
    render_state.min_lod = selection.min_lod_selected;
    render_state.max_lod = selection.max_lod_selected;

    let desired: HashMap<TileId, CoarserEdges> = selection
        .tiles
        .iter()
        .map(|t| (t.id, t.coarser_edges))
        .collect();

    // Despawn tiles no longer selected.
    let stale_ids: Vec<TileId> = render_state
        .tiles
        .keys()
        .copied()
        .filter(|id| !desired.contains_key(id))
        .collect();
    for id in stale_ids {
        if let Some(removed) = render_state.tiles.remove(&id) {
            commands.entity(removed.entity).despawn();
            meshes.remove(&removed.mesh);
        }
    }

    // Spawn or rebuild tiles whose edge stitching or LocalOrigin changed.
    let origin = origin_state.origin;
    for selected in &selection.tiles {
        let tile_id = selected.id;
        let coarser = selected.coarser_edges;
        if let Some(existing) = render_state.tiles.get_mut(&tile_id) {
            if existing.coarser_edges != coarser || existing.baked_origin != origin {
                let render_mesh = build_tile_render_mesh(planet, tile_id, coarser, origin);
                if let Some(mut mesh_asset) = meshes.get_mut(&existing.mesh) {
                    *mesh_asset = to_bevy_mesh(render_mesh);
                }
                existing.coarser_edges = coarser;
                existing.baked_origin = origin;
            }
        } else {
            let render_mesh = build_tile_render_mesh(planet, tile_id, coarser, origin);
            let mesh_handle = meshes.add(to_bevy_mesh(render_mesh));
            let mat_handle = render_state.face_materials[tile_id.face.index()].clone();
            let entity = commands
                .spawn((
                    Mesh3d(mesh_handle.clone()),
                    MeshMaterial3d(mat_handle),
                    Transform::IDENTITY,
                ))
                .id();
            render_state.tiles.insert(
                tile_id,
                CachedTile {
                    entity,
                    mesh: mesh_handle,
                    coarser_edges: coarser,
                    baked_origin: origin,
                },
            );
        }
    }
}

/// Draws great-circle cube-face edge highlights using `Gizmos` with altitude-aware
/// subdivision and lift so 2-face seams and 3-face corners stay visible above the sphere
/// both from orbit and at `20 m` local ground scale.
#[expect(
    clippy::needless_pass_by_value,
    reason = "Bevy ECS system parameters require Res<T> by value"
)]
pub fn draw_cube_face_seams(
    mut gizmos: Gizmos,
    world_view: Res<ClientWorldView>,
    cam_state: Res<GlobeCameraState>,
    origin_state: Res<OriginState>,
) {
    let planet = world_view.snapshot().planet;
    let r = planet.radius_meters();
    let alt = cam_state.altitude_meters.max(1.0);
    let origin = origin_state.origin;
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
            // Restrict subdivision to a local window around the focus projection so chord sagitta stays sub-millimeter.
            let focus_cube = focus.scale(1.0 / focus.x.abs().max(focus.y.abs()).max(focus.z.abs()));
            let t_proj = ((focus_cube - a).dot(dir) / dir.dot(dir)).clamp(0.0, 1.0);
            let half_window = ((alt * 12.0) / r).clamp(1e-6, 0.5);
            let lo = (t_proj - half_window).max(0.0);
            let hi = (t_proj + half_window).min(1.0);
            let closest = (a + dir.scale(t_proj)).normalized();
            if let Ok(c_unit) = closest
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
            let raw = a + dir.scale(t);
            if let Ok(unit) = raw.normalized() {
                let pt = origin.to_render_vec3(unit.scale(r + lift));
                let v = Vec3::new(pt.x, pt.y, pt.z);
                if let Some(p) = prev {
                    gizmos.line(p, v, seam_color);
                }
                prev = Some(v);
            }
        }
    }
}

fn to_bevy_mesh(tile_mesh: TileRenderMesh) -> Mesh {
    let mut mesh = Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::default(),
    );
    mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, tile_mesh.positions);
    mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, tile_mesh.normals);
    mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, tile_mesh.uvs);
    mesh.insert_indices(Indices::U32(tile_mesh.indices));
    mesh
}

fn build_tile_grid_image() -> Image {
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
