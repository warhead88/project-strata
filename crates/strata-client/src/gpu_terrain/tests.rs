mod gradients;
mod selection;
mod transition;

use bevy::render::{
    render_resource::{
        BufferBindingType, CommandEncoderDescriptor, ComputePassDescriptor, MapMode, PollType,
        RawComputePipelineDescriptor, ShaderStages,
    },
    renderer::initialize_renderer,
    settings::{Backends, RenderResources, WgpuSettings},
};
use bytemuck::Pod;
use strata_globe::{CubeFace, EcefMeters, FaceUv, ecef_to_face_uv_unit, face_uv_to_unit};
use strata_terrain::{WaterBodyKind, lat_lon_to_unit};

use super::{
    pipeline::{CUBE, EARTH, SURFACE, pipeline_layout, shader, storage_group, surface_group},
    source::meters_f32,
    view::face_anchor,
    *,
};

fn input(lat: f64, lon: f64, altitude: f64) -> Result<ExtractedTerrain, strata_globe::GlobeError> {
    let terrain = TerrainElevation::earth_default();
    let mut camera = GlobeCameraState::new(PlanetSpec::earth(), lat_lon_to_unit(lat, lon))?;
    camera.altitude_meters = altitude;
    camera.pitch_rad = 0.0;
    camera.sync_terrain_elevations(terrain.sampler());
    Ok(ExtractedTerrain {
        camera,
        terrain,
        debug: false,
    })
}

#[test]
fn reusable_grid_has_outward_winding_and_shared_boundary() {
    let n = PlanetSpec::earth().grid_segments();
    let indices = grid_indices(n);
    assert_eq!(indices.len(), (n * n * 6) as usize);
    let xy = |i: u32| (i % (n + 1), i / (n + 1));
    for tri in indices.chunks_exact(3) {
        let points = [xy(tri[0]), xy(tri[1]), xy(tri[2])];
        assert!(points.iter().all(|&(x, y)| x <= n && y <= n));
        let signed_area = |(x, y): (u32, u32), (xx, yy): (u32, u32)| {
            i64::from(x) * i64::from(yy) - i64::from(y) * i64::from(xx)
        };
        assert_eq!(
            signed_area(points[0], points[1])
                + signed_area(points[1], points[2])
                + signed_area(points[2], points[0]),
            1
        );
    }
}

#[test]
#[ignore = "requires a native wgpu adapter; run explicitly with WGPU_BACKEND=metal on macOS"]
fn native_gpu_terrain_matches_world_and_stitches_balanced_selection()
-> Result<(), Box<dyn std::error::Error>> {
    let settings = WgpuSettings::default();
    let RenderResources(device, queue, info, ..) = bevy::tasks::block_on(initialize_renderer(
        settings.backends.unwrap_or(Backends::PRIMARY),
        None,
        &settings,
    ));
    println!("GPU test on {} ({:?})", info.name, info.backend);
    let input = input(33.45, 45.0, 20.0)?;
    let world = input.terrain.world().ok_or("test world missing")?;
    let data = world.surface_data().ok_or("test surface missing")?;
    let source = SourceGpu::upload(&device, &queue, &data).ok_or("test upload failed")?;
    gradients::validate(
        &device,
        &queue,
        data.detail_octaves(),
        &source.gradients,
        source.settings.second_octave_offset,
    )?;
    let wrapped_seed_octaves = [(3, u64::MAX - 3), (2, 0x8000_0000_ffff_ffff)];
    let wrapped = super::gradients::generate(&device, &queue, wrapped_seed_octaves);
    gradients::validate(
        &device,
        &queue,
        wrapped_seed_octaves,
        &wrapped.buffer,
        wrapped.second_octave_offset,
    )?;
    let gpu = TerrainGpu::new(&device, input.camera.planet, source)
        .ok_or("terrain GPU allocation failed")?;
    validate_surface(&device, &queue, &input, &gpu)?;
    selection::validate(&device, &queue, &input, &gpu)?;
    Ok(())
}

const PROBE: &str = r"
struct Point { u: f32, v: f32, face: u32, pad: u32 }
@group(1) @binding(0) var<storage, read> points: array<Point>;
@group(1) @binding(1) var<storage, read_write> output: array<vec4<f32>>;

@compute @workgroup_size(32)
fn probe(@builtin(global_invocation_id) index: vec3<u32>) {
    let i = index.x;
    if i >= arrayLength(&points) { return; }
    let point = points[i];
    let uv = vec2(point.u, point.v);
    let dir = direction(point.face, uv);
    let s = sample_surface(point.face, uv, dir);
    output[2u*i] = vec4(s.height, s.level, s.kind, 0.0);
    output[2u*i+1u] = vec4(relative_on_sphere(point.face, uv), 0.0);
}
";

fn validate_surface(
    device: &RenderDevice,
    queue: &RenderQueue,
    input: &ExtractedTerrain,
    terrain: &TerrainGpu,
) -> Result<(), Box<dyn std::error::Error>> {
    let world = input.terrain.world().ok_or("test world missing")?;
    let source = &terrain.source;
    let (eye, _, _) = input.camera.compute_pose_f64();
    let coords = probe_points(world, &input.camera, eye)?;
    let points: Vec<_> = coords
        .iter()
        .copied()
        .map(ecef_to_face_uv_unit)
        .map(|(face, uv)| {
            [
                meters_f32(uv.u).to_bits(),
                meters_f32(uv.v).to_bits(),
                u32::try_from(face.index()).unwrap_or(0),
                0,
            ]
        })
        .collect();
    let output_len = u64::try_from(points.len())? * 32;
    let request = initialized(
        device,
        "Terrain probe inputs",
        BufferUsages::STORAGE,
        &points,
    );
    let output = allocate(
        device,
        "Terrain probe outputs",
        output_len,
        BufferUsages::STORAGE | BufferUsages::COPY_SRC,
    );
    let readback = allocate(
        device,
        "Terrain probe readback",
        output_len,
        BufferUsages::COPY_DST | BufferUsages::MAP_READ,
    );
    let (world_layout, world_group) =
        surface_group(device, &terrain.frame, source, ShaderStages::COMPUTE);
    let (probe_layout, probe_group) = storage_group(
        device,
        "Terrain probe",
        ShaderStages::COMPUTE,
        [
            (&request, BufferBindingType::Storage { read_only: true }),
            (&output, BufferBindingType::Storage { read_only: false }),
        ],
    );
    let module = shader(
        device,
        "Terrain parity probe",
        &[CUBE, EARTH, SURFACE, PROBE],
    );
    let layout = pipeline_layout(device, &world_layout, &probe_layout);
    let pipeline = device.create_compute_pipeline(&RawComputePipelineDescriptor {
        label: Some("Terrain parity probe"),
        layout: Some(&layout),
        module: &module,
        entry_point: Some("probe"),
        compilation_options: default(),
        cache: None,
    });
    let frame = input.frame(Mat4::IDENTITY, 623.0, source.settings);
    queue.write_buffer(&terrain.frame, 0, bytes_of(&frame));
    let mut encoder = device.create_command_encoder(&CommandEncoderDescriptor::default());
    {
        let mut pass = encoder.begin_compute_pass(&ComputePassDescriptor::default());
        pass.set_pipeline(&pipeline);
        pass.set_bind_group(0, &*world_group, &[]);
        pass.set_bind_group(1, &*probe_group, &[]);
        pass.dispatch_workgroups(u32::try_from(coords.len())?.div_ceil(32), 1, 1);
    }
    encoder.copy_buffer_to_buffer(&output, 0, &readback, 0, output_len);
    queue.submit([encoder.finish()]);
    let data = read::<[f32; 4]>(device, &readback)?;
    assert_parity(
        world,
        input.camera.planet,
        eye,
        &frame,
        &coords,
        &points,
        &data,
    );
    Ok(())
}

fn probe_points(
    world: &strata_terrain::TerrainWorld,
    camera: &GlobeCameraState,
    eye: EcefMeters,
) -> Result<Vec<EcefMeters>, strata_globe::GlobeError> {
    let mut coords = vec![
        lat_lon_to_unit(46.54, 10.12),
        lat_lon_to_unit(27.99, 86.92),
        lat_lon_to_unit(41.0, 51.0),
        lat_lon_to_unit(32.0, 32.0),
        lat_lon_to_unit(31.05, 30.99),
        lat_lon_to_unit(10.0, 179.9999),
        lat_lon_to_unit(10.0, -179.9999),
        lat_lon_to_unit(89.999, 0.0),
        lat_lon_to_unit(-89.999, 40.0),
    ];
    for river in world.rivers() {
        for pair in river.waypoints.windows(2) {
            coords.push((pair[0].unit_dir + pair[1].unit_dir).normalized()?);
        }
    }
    for face in [CubeFace::PosX, CubeFace::PosZ] {
        let anchor = face_anchor(face, eye, camera.focus_unit, camera.planet.radius_meters());
        for du in [-2e-6, 0.0, 2e-6] {
            for dv in [-3e-6, 0.0, 3e-6] {
                coords.push(face_uv_to_unit(
                    face,
                    FaceUv::clamped(f64::from(anchor.uv[0]) + du, f64::from(anchor.uv[1]) + dv),
                ));
            }
        }
    }
    Ok(coords)
}

fn assert_parity(
    world: &strata_terrain::TerrainWorld,
    planet: PlanetSpec,
    eye: EcefMeters,
    frame: &GpuView,
    coords: &[EcefMeters],
    points: &[[u32; 4]],
    data: &[[f32; 4]],
) {
    let mut max_height_error = 0.0_f64;
    let mut max_position_error = 0.0_f64;
    let mut shoreline_differences = 0;
    let mut displaced_shore = 0;
    for ((&original, point), result) in coords.iter().zip(points).zip(data.chunks_exact(2)) {
        let face = CubeFace::ALL[point[2] as usize];
        let uv = FaceUv {
            u: f64::from(f32::from_bits(point[0])),
            v: f64::from(f32::from_bits(point[1])),
        };
        let dir = face_uv_to_unit(face, uv);
        let height = world.sample_elevation_unit_prevalidated(dir);
        let height_error = (height - f64::from(result[0][0])).abs();
        max_height_error = max_height_error.max(height_error);
        let water = world.sample_water(dir);
        let kind = gpu_water_kind(water);
        if (result[0][2] - kind).abs() > f32::EPSILON {
            shoreline_differences += 1;
            let (east, north) = strata_globe::canonical_tangent_basis(dir);
            let located = (-2..=2).any(|x| {
                (-2..=2).any(|y| {
                    let nearby = dir
                        + (east.scale(f64::from(x)) + north.scale(f64::from(y)))
                            .scale(3.0 / planet.radius_meters());
                    (gpu_water_kind(world.sample_water(nearby)) - result[0][2]).abs() < f32::EPSILON
                })
            });
            if !located {
                displaced_shore += 1;
                if displaced_shore <= 5 {
                    eprintln!("Water mismatch at {original:?}: {water:?}, {result:?}");
                }
            }
        }
        let expected = dir.scale(planet.radius_meters()) - eye;
        if expected.norm() < 500.0 {
            let [x, y, z, _] = result[1].map(f64::from);
            let error = expected.distance_to(EcefMeters::from_raw(x, y, z));
            max_position_error = max_position_error.max(error);
            assert!(
                error < 0.003,
                "{face:?} {uv:?} anchor {:?} {:?} {expected:?} {result:?}, error {error}",
                frame.anchors[face.index()].uv,
                frame.anchors[face.index()].relative
            );
        }
    }
    println!(
        "Maximum sample height difference {max_height_error:.4} m, near-eye position difference {max_position_error:.6} m; {shoreline_differences}/{} shore classifications differed, {displaced_shore} beyond the ±6 m neighborhood",
        coords.len()
    );
    assert!(max_height_error < 5.0);
    assert_eq!(displaced_shore, 0);
}

fn gpu_water_kind(water: strata_terrain::WaterSample) -> f32 {
    match water.kind {
        WaterBodyKind::Dry => 0.0,
        WaterBodyKind::Ocean => 1.0,
        WaterBodyKind::Lake(_) if water.is_freshwater => 2.0,
        WaterBodyKind::Lake(_) => 3.0,
        WaterBodyKind::River(_) => 4.0,
    }
}

fn read<T: Pod>(
    device: &RenderDevice,
    readback: &Buffer,
) -> Result<Vec<T>, Box<dyn std::error::Error>> {
    let (tx, rx) = std::sync::mpsc::channel();
    let slice = readback.slice(..);
    device.map_buffer(&slice, MapMode::Read, move |result| {
        let _ = tx.send(result);
    });
    device.poll(PollType::wait_indefinitely())?;
    rx.recv()??;
    let data = bytemuck::cast_slice(&slice.get_mapped_range()?).to_vec();
    readback.unmap();
    Ok(data)
}

#[test]
#[ignore = "measures release source preparation on a native adapter"]
fn native_gpu_source_preparation() -> Result<(), Box<dyn std::error::Error>> {
    let settings = WgpuSettings::default();
    let RenderResources(device, queue, info, ..) = bevy::tasks::block_on(initialize_renderer(
        settings.backends.unwrap_or(Backends::PRIMARY),
        None,
        &settings,
    ));
    println!("Source preparation on {} ({:?})", info.name, info.backend);
    let world = strata_terrain::TerrainWorld::earth_default();
    let source = world.surface_data().ok_or("test surface missing")?;
    let mut cpu = Vec::new();
    let mut total = Vec::new();
    for run in 0..9 {
        let start = std::time::Instant::now();
        let resident = SourceGpu::upload(&device, &queue, &source).ok_or("source upload failed")?;
        let cpu_ms = start.elapsed().as_secs_f64() * 1000.0;
        queue.submit([]);
        device.poll(PollType::wait_indefinitely())?;
        let total_ms = start.elapsed().as_secs_f64() * 1000.0;
        if run == 0 {
            println!("cold CPU {cpu_ms:.3} ms, ready {total_ms:.3} ms");
        } else {
            cpu.push(cpu_ms);
            total.push(total_ms);
        }
        drop(resident);
    }
    cpu.sort_by(f64::total_cmp);
    total.sort_by(f64::total_cmp);
    println!(
        "warm medians CPU {:.3} ms, ready {:.3} ms",
        f64::midpoint(cpu[3], cpu[4]),
        f64::midpoint(total[3], total[4])
    );
    Ok(())
}
