use std::{
    sync::Arc,
    time::{Duration, Instant},
};

use bevy::{
    app::PluginsState,
    camera::RenderTarget,
    log::LogPlugin,
    render::{pipelined_rendering::PipelinedRenderingPlugin, renderer::RenderAdapterInfo},
    winit::WinitPlugin,
};
use strata_globe::FlatElevation;
use strata_terrain::{TerrainConfig, TerrainPatchKind, TerrainWorld};

use crate::{camera::OriginState, globe_render::GlobeRenderState};

use super::*;

#[test]
#[ignore = "renders an offscreen Bevy view on a native adapter"]
fn native_gpu_terrain_transitions_retire_inactive_surfaces()
-> Result<(), Box<dyn std::error::Error>> {
    let initial = input(46.54, 10.12, 9e6)?;
    let earth = initial
        .terrain
        .world()
        .ok_or("missing initial world")?
        .clone();
    let mut app = client(initial)?;
    app.update();
    assert_gpu(&app, PlanetSpec::earth(), 0);
    let gpu = terrain_state(&app)
        .active
        .as_ref()
        .ok_or("missing GPU terrain")?;
    let gradients = gpu.source.gradients.id();
    let draw_args = gpu.indirect.id();

    // A camera update without a change of terrain or planet must reuse the GPU state.
    app.update();
    assert_eq!(
        terrain_state(&app)
            .active
            .as_ref()
            .map(|gpu| gpu.indirect.id()),
        Some(draw_args)
    );

    // A compatible planet change refreshes draw buffers while keeping the original source.
    let compatible = PlanetSpec::new(6_200_000.0, 16, 8)?;
    app.world_mut().resource_mut::<GlobeCameraState>().planet = compatible;
    app.update();
    assert_gpu(&app, compatible, 0);
    let gpu = terrain_state(&app)
        .active
        .as_ref()
        .ok_or("missing resized GPU terrain")?;
    assert_eq!(gpu.source.gradients.id(), gradients);
    assert_ne!(gpu.indirect.id(), draw_args);

    // An incompatible grid falls back to the CPU and the old GPU draw is retired.
    app.world_mut().resource_mut::<GlobeCameraState>().planet =
        PlanetSpec::new(6_200_000.0, 16, 10)?;
    let cpu_tiles = await_cpu_meshes(&mut app)?;
    assert_eq!(
        terrain_state(&app)
            .resident
            .as_ref()
            .map(|(_, source)| source.gradients.id()),
        Some(gradients)
    );
    app.world_mut().resource_mut::<GlobeCameraState>().planet = PlanetSpec::earth();
    app.update();
    assert_gpu(&app, PlanetSpec::earth(), 0);
    assert_retired_meshes(&app, &cpu_tiles);

    let mut patched = (*earth).clone();
    patched.apply_patch(TerrainPatchKind::FlattenPad {
        center_unit: lat_lon_to_unit(46.54, 10.12),
        radius_m: 25.0,
        blend_width_m: 10.0,
        target_elevation_m: 2400.0,
    })?;
    app.world_mut()
        .resource_mut::<TerrainElevation>()
        .set_world(Arc::new(patched));
    let patched_tiles = await_cpu_meshes(&mut app)?;
    assert!(terrain_state(&app).resident.is_none());

    app.world_mut()
        .resource_mut::<TerrainElevation>()
        .set_world(Arc::clone(&earth));
    app.update();
    assert_gpu(&app, PlanetSpec::earth(), 2);
    assert_retired_meshes(&app, &patched_tiles);

    app.world_mut()
        .resource_mut::<TerrainElevation>()
        .set_sampler(Arc::new(FlatElevation));
    let sampled_tiles = await_cpu_meshes(&mut app)?;
    assert!(terrain_state(&app).resident.is_none());
    app.world_mut()
        .resource_mut::<TerrainElevation>()
        .set_world(earth);
    app.update();
    assert_gpu(&app, PlanetSpec::earth(), 4);
    assert_retired_meshes(&app, &sampled_tiles);

    check_supported_replacement_and_cpu_preference(&mut app, compatible)?;
    Ok(())
}

fn check_supported_replacement_and_cpu_preference(
    app: &mut App,
    compatible: PlanetSpec,
) -> Result<(), Box<dyn std::error::Error>> {
    let gradients = terrain_state(app)
        .active
        .as_ref()
        .ok_or("missing GPU source before replacement")?
        .source
        .gradients
        .id();
    let changed = Arc::new(TerrainWorld::new(TerrainConfig::earth(u64::MAX))?);
    app.world_mut()
        .resource_mut::<TerrainElevation>()
        .set_world(Arc::clone(&changed));
    app.update();
    assert_gpu(app, PlanetSpec::earth(), 5);
    let gpu = terrain_state(app)
        .active
        .as_ref()
        .ok_or("missing replacement GPU source")?;
    assert_ne!(gpu.source.gradients.id(), gradients);

    // The command-line preference is sticky even when an eligible source is installed.
    app.world_mut().insert_resource(TerrainBackend::new(false));
    app.update();
    assert!(!app.world().resource::<TerrainBackend>().uses_gpu());
    assert!(terrain_state(app).active.is_some());
    assert!(!app.world().contains_resource::<GlobeRenderState>());
    let world = app.world_mut();
    world.resource_mut::<TerrainElevation>().set_world(changed);
    world.resource_mut::<GlobeCameraState>().planet = compatible;
    app.update();
    assert!(!app.world().resource::<TerrainBackend>().uses_gpu());
    assert!(terrain_state(app).active.is_some());
    assert!(terrain_state(app).resident.is_some());
    Ok(())
}

fn client(initial: ExtractedTerrain) -> Result<App, strata_globe::GlobeError> {
    let (eye, _, _) = initial.camera.compute_pose_f64();
    let mut app = App::new();
    app.add_plugins(
        DefaultPlugins
            .set(WindowPlugin {
                primary_window: None,
                ..default()
            })
            .disable::<WinitPlugin>()
            .disable::<PipelinedRenderingPlugin>()
            .disable::<LogPlugin>(),
    )
    .add_plugins(TerrainRenderPlugin)
    .insert_resource(TerrainBackend::new(true))
    .insert_resource(initial.camera)
    .insert_resource(initial.terrain)
    .insert_resource(OriginState::new(eye)?)
    .init_resource::<TerrainDebug>();
    let target = app
        .world_mut()
        .resource_mut::<Assets<Image>>()
        .add(Image::new_target_texture(
            128,
            72,
            TextureFormat::Rgba8UnormSrgb,
            None,
        ));
    app.world_mut().spawn((
        Camera3d::default(),
        RenderTarget::Image(target.into()),
        Msaa::Off,
    ));
    while app.plugins_state() == PluginsState::Adding {
        std::thread::yield_now();
    }
    app.finish();
    app.cleanup();
    let info = app.world().resource::<RenderAdapterInfo>();
    println!("Terrain transitions on {} ({:?})", info.name, info.backend);
    Ok(app)
}

fn terrain_state(app: &App) -> &PreparedTerrain {
    app.sub_app(RenderApp).world().resource::<PreparedTerrain>()
}

fn assert_gpu(app: &App, planet: PlanetSpec, revision: u64) {
    assert!(app.world().resource::<TerrainBackend>().uses_gpu());
    assert!(!app.world().contains_resource::<GlobeRenderState>());
    let prepared = terrain_state(app);
    assert!(
        prepared
            .requested
            .is_some_and(|key| key.planet == planet && key.revision == revision)
    );
    assert!(
        prepared
            .active
            .as_ref()
            .is_some_and(|gpu| !gpu.pipelines.is_empty())
    );
}

fn await_cpu_meshes(app: &mut App) -> Result<Vec<(Entity, Handle<Mesh>)>, &'static str> {
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        app.update();
        assert!(!app.world().resource::<TerrainBackend>().uses_gpu());
        assert!(terrain_state(app).active.is_none());
        assert!(app.world().contains_resource::<GlobeRenderState>());
        let world = app.world_mut();
        let tiles: Vec<_> = world
            .query::<(Entity, &Mesh3d)>()
            .iter(world)
            .map(|(e, mesh)| (e, mesh.0.clone()))
            .collect();
        if !tiles.is_empty() {
            return Ok(tiles);
        }
        std::thread::yield_now();
    }
    Err("CPU terrain tasks did not produce a visible mesh")
}

fn assert_retired_meshes(app: &App, tiles: &[(Entity, Handle<Mesh>)]) {
    let assets = app.world().resource::<Assets<Mesh>>();
    for (entity, mesh) in tiles {
        assert!(app.world().get_entity(*entity).is_err());
        assert!(assets.get(mesh).is_none());
    }
}
