use bevy::{camera::CameraProjection, render::render_resource::PipelineCompilationOptions};
use strata_globe::{TileEdge, TileId, TileUv, ecef_to_tile_uv};

use super::{super::pipeline::DRAW, *};

const GEOMETRY: &str = r"
@group(1) @binding(1) var<storage, read_write> positions: array<vec4<f32>>;

@compute @workgroup_size(64)
fn geometry(@builtin(global_invocation_id) id: vec3<u32>) {
    let side = view.settings.z + 1u;
    let count = side * side;
    if id.x < count * arrayLength(&tiles) {
        let vertex = raster_vertex(tiles[id.x / count], id.x % count);
        positions[id.x] = vec4(vertex.relative, 1.0);
    }
}
";

type Instance = [u32; 4];
type ProbeResult<T> = Result<T, Box<dyn std::error::Error>>;

struct CapturedView {
    eye: EcefMeters,
    frame: GpuView,
    tiles: Vec<Instance>,
}

pub(super) fn validate(
    device: &RenderDevice,
    queue: &RenderQueue,
    gpu: &TerrainGpu,
) -> ProbeResult<()> {
    for (label, lat, lon, lod, pitch) in [
        ("Alpine slope", 46.54, 10.12, 15, 28.0_f64),
        ("Kerch shore", 45.335_416_667, 36.579_166_667, 13, 15.0),
    ] {
        let mut scene = input(lat, lon, 1000.0)?;
        scene.camera.pitch_rad = pitch.to_radians();
        let (focus, _, _) = ecef_to_tile_uv(scene.camera.planet, scene.camera.focus_unit, lod)?;
        if label == "Kerch shore" {
            assert_shore(&scene, focus)?;
        }
        let segments = scene.camera.planet.grid_segments();
        let target = scene.camera.planet.radius_meters() * std::f64::consts::FRAC_PI_2
            / f64::from((1 << lod) * segments)
            * 0.30
            * 623.0
            / 3.5;
        let altitude = target / scene.camera.pitch_rad.cos();
        let capture = |altitude| scene_at(device, queue, gpu, &mut scene, altitude);
        let (before, after) = bracket(capture, focus, altitude)?;
        let children = focus.children().ok_or("expected child tiles")?;
        let neighbor = focus.neighbor(TileEdge::Right).tile;
        assert_eq!(focus.face, neighbor.face);
        let neighbors = neighbor
            .children()
            .ok_or("expected neighboring child tiles")?;
        let parent = instance(&before.tiles, focus).ok_or("parent missing before split")?;
        let neighbor_parent =
            instance(&before.tiles, neighbor).ok_or("neighbor missing before split")?;
        let after_tiles = children
            .into_iter()
            .chain([neighbors[0], neighbors[2]])
            .map(|tile| instance(&after.tiles, tile).ok_or("expected child to replace parent"));
        let mut after_tiles: Vec<_> = after_tiles.collect::<Result<_, _>>()?;
        let stitched = after
            .tiles
            .iter()
            .copied()
            .find(|tile| tile[2] == u32::from(lod) + 1 && tile[3] >> 3 != 0)
            .ok_or("no stitched child was drawn")?;
        after_tiles.push(stitched);
        let prev = positions(
            device,
            queue,
            gpu,
            &before.frame,
            &[parent, neighbor_parent],
        )?;
        let next = positions(device, queue, gpu, &after.frame, &after_tiles)?;
        let (geometry, stitched_geometry) = next.split_at(6 * (segments as usize + 1).pow(2));
        let change = measure(&before, &after, segments, &prev, geometry);
        let stitched_error =
            stitched_midpoints(&after.frame, segments, stitched[3] >> 3, stitched_geometry);
        println!(
            "{label}, LOD {lod} → {}, eye delta {:.4} m, maximum jump {:.5} px, shared edge {:.5} px, stitched edge {stitched_error:.5} px",
            lod + 1,
            before.eye.distance_to(after.eye),
            change[0],
            change[1]
        );
        assert!(change[0] < 0.15 && change[1] < 0.05, "{label}: {change:?}");
        assert!(
            stitched_error < 0.05,
            "{label}: stitched edge {stitched_error} px"
        );
    }
    Ok(())
}

fn assert_shore(scene: &ExtractedTerrain, focus: TileId) -> ProbeResult<()> {
    let world = scene
        .terrain
        .world()
        .ok_or("shore probe needs the Earth world")?;
    let mut dry = false;
    let mut ocean = false;
    for y in 0..=16 {
        for x in 0..=16 {
            let uv =
                focus.tile_uv_to_face_uv(TileUv::clamped(f64::from(x) / 16.0, f64::from(y) / 16.0));
            let kind = world.sample_water(face_uv_to_unit(focus.face, uv)).kind;
            dry |= kind == WaterBodyKind::Dry;
            ocean |= kind == WaterBodyKind::Ocean;
        }
    }
    assert!(
        dry && ocean,
        "the Kerch LOD tile must straddle the shoreline: {focus:?}"
    );
    Ok(())
}

fn stitched_midpoints(frame: &GpuView, segments: u32, edges: u32, pos: &[[f32; 4]]) -> f32 {
    let n = segments as usize;
    let side = n + 1;
    let clip = Mat4::from_cols_array_2d(&frame.clip_from_eye);
    let project = |v: Vec4| {
        let p = clip * v;
        p.truncate().truncate() / p.w * Vec2::new(640.0, 360.0)
    };
    let mut max_error = 0.0_f32;
    for (bit, (start, stride)) in [(0, side), (n, side), (0, 1), (n * side, 1)]
        .into_iter()
        .enumerate()
    {
        if edges & (1 << bit) == 0 {
            continue;
        }
        for step in (1..n).step_by(2) {
            let index = start + step * stride;
            let [previous, midpoint, next] =
                [index - stride, index, index + stride].map(|j| Vec4::from_array(pos[j]));
            max_error = max_error.max(project(midpoint).distance(project((previous + next) * 0.5)));
        }
    }
    max_error
}

fn instance(tiles: &[Instance], tile: TileId) -> Option<Instance> {
    tiles.iter().copied().find(|raw| {
        raw[..3] == [tile.x, tile.y, u32::from(tile.lod)]
            && (raw[3] & 7) == u32::from(tile.face as u8)
    })
}

fn bracket(
    mut capture: impl FnMut(f64) -> ProbeResult<CapturedView>,
    focus: TileId,
    altitude: f64,
) -> ProbeResult<(CapturedView, CapturedView)> {
    let (mut low, mut high) = (altitude * 0.85, altitude * 1.15);
    let (mut after, mut before) = (capture(low)?, capture(high)?);
    assert!(instance(&before.tiles, focus).is_some());
    assert!(instance(&after.tiles, focus).is_none());
    while high - low > 0.002 {
        let mid = f64::midpoint(low, high);
        let candidate = capture(mid)?;
        if instance(&candidate.tiles, focus).is_some() {
            (high, before) = (mid, candidate);
        } else {
            (low, after) = (mid, candidate);
        }
    }
    Ok((before, after))
}

fn scene_at(
    device: &RenderDevice,
    queue: &RenderQueue,
    gpu: &TerrainGpu,
    scene: &mut ExtractedTerrain,
    altitude: f64,
) -> ProbeResult<CapturedView> {
    scene.camera.altitude_meters = altitude;
    scene
        .camera
        .sync_terrain_elevations(scene.terrain.sampler());
    let (eye, target, up) = scene.camera.compute_pose_f64();
    let as_vec3 = |v: EcefMeters| Vec3::new(meters_f32(v.x), meters_f32(v.y), meters_f32(v.z));
    let projection = PerspectiveProjection {
        fov: 60.0_f32.to_radians(),
        aspect_ratio: 16.0 / 9.0,
        near: 0.4,
        ..default()
    }
    .get_clip_from_view();
    let rotation = Transform::IDENTITY
        .looking_at(as_vec3(target - eye), as_vec3(up))
        .rotation
        .inverse();
    let frame = scene.frame(
        projection * Mat4::from_quat(rotation),
        623.0,
        gpu.source.settings,
    );
    let tile_bytes = TILE_CAPACITY * 16;
    let readback = allocate(
        device,
        "LOD selected tiles",
        DRAW_BYTES + tile_bytes,
        BufferUsages::MAP_READ | BufferUsages::COPY_DST,
    );
    queue.write_buffer(&gpu.frame, 0, bytes_of(&frame));
    let mut encoder = device.create_command_encoder(&CommandEncoderDescriptor::default());
    gpu.encode_selection(&mut encoder, frame.settings[0], None);
    encoder.copy_buffer_to_buffer(&gpu.indirect, 0, &readback, 0, DRAW_BYTES);
    encoder.copy_buffer_to_buffer(&gpu.land, 0, &readback, DRAW_BYTES, tile_bytes);
    queue.submit([encoder.finish()]);
    let values = read::<u32>(device, &readback)?;
    let count = values[1] as usize;
    let tiles = values[5..5 + 4 * count].as_chunks::<4>().0.to_vec();
    Ok(CapturedView { eye, frame, tiles })
}

fn positions(
    device: &RenderDevice,
    queue: &RenderQueue,
    gpu: &TerrainGpu,
    frame: &GpuView,
    tiles: &[Instance],
) -> ProbeResult<Vec<[f32; 4]>> {
    let count = (frame.settings[2] + 1).pow(2) * u32::try_from(tiles.len())?;
    let size = u64::from(count) * 16;
    let request = initialized(device, "LOD probe tiles", BufferUsages::STORAGE, tiles);
    let output = allocate(
        device,
        "LOD probe points",
        size,
        BufferUsages::STORAGE | BufferUsages::COPY_SRC,
    );
    let readback = allocate(
        device,
        "LOD probe points readback",
        size,
        BufferUsages::MAP_READ | BufferUsages::COPY_DST,
    );
    let stage = ShaderStages::COMPUTE;
    let (world_layout, world) = surface_group(device, &gpu.frame, &gpu.source, stage);
    let (sample_layout, sample) = storage_group(
        device,
        "LOD geometry",
        stage,
        [
            (&request, BufferBindingType::Storage { read_only: true }),
            (&output, BufferBindingType::Storage { read_only: false }),
        ],
    );
    let module = shader(
        device,
        "LOD vertex geometry",
        &[CUBE, EARTH, LOCAL, SURFACE, DRAW, GEOMETRY],
    );
    let layout = pipeline_layout(device, &world_layout, &sample_layout);
    let pipeline = device.create_compute_pipeline(&RawComputePipelineDescriptor {
        label: Some("LOD vertex geometry"),
        layout: Some(&layout),
        module: &module,
        entry_point: Some("geometry"),
        compilation_options: PipelineCompilationOptions::default(),
        cache: None,
    });
    queue.write_buffer(&gpu.frame, 0, bytes_of(frame));
    let mut encoder = device.create_command_encoder(&CommandEncoderDescriptor::default());
    {
        let mut pass = encoder.begin_compute_pass(&ComputePassDescriptor::default());
        pass.set_pipeline(&pipeline);
        pass.set_bind_group(0, &*world, &[]);
        pass.set_bind_group(1, &*sample, &[]);
        pass.dispatch_workgroups(count.div_ceil(64), 1, 1);
    }
    encoder.copy_buffer_to_buffer(&output, 0, &readback, 0, size);
    queue.submit([encoder.finish()]);
    read(device, &readback)
}

fn measure(
    before: &CapturedView,
    after: &CapturedView,
    segments: u32,
    prev: &[[f32; 4]],
    next: &[[f32; 4]],
) -> [f32; 2] {
    let n = segments as usize;
    let side = n + 1;
    let per_tile = side * side;
    // Reproject both meshes from the exact same eye, preserving the positions
    // generated with each frame's own selection and morph weights.
    let (old, new) = (
        Mat4::from_cols_array_2d(&before.frame.clip_from_eye),
        after.eye - before.eye,
    );
    let delta = Vec3::new(meters_f32(new.x), meters_f32(new.y), meters_f32(new.z));
    let project = |relative: Vec3| {
        let clip = old * relative.extend(1.0);
        clip.truncate().truncate() / clip.w * Vec2::new(640.0, 360.0)
    };
    let from = |samples: &[[f32; 4]], index| Vec4::from_array(samples[index]).truncate();
    let mut peak = 0.0_f32;
    let mut edge_peak = 0.0_f32;
    for (child, samples) in next.chunks_exact(per_tile).enumerate() {
        let quadrant = [0, 1, 2, 3, 0, 2][child];
        for y in 0..=n {
            for x in 0..=n {
                let fx = (quadrant % 2) * n + x;
                let fy = (quadrant / 2) * n + y;
                let (col, row) = (fx / 2, fy / 2);
                let coarse_index = usize::from(child >= 4) * per_tile + row * side + col;
                let start = from(prev, coarse_index);
                // Every fine vertex is either a coarse vertex or a midpoint of
                // one coarse edge or diagonal. Compare the actual geometric surfaces.
                let end = from(prev, coarse_index + (fx & 1) + (fy & 1) * side);
                let exact = from(samples, y * side + x) + delta;
                peak = peak.max(project(exact).distance(project((start + end) * 0.5)));
                if (child == 1 || child == 3) && x == n {
                    let other = from(next, (4 + child / 2) * per_tile + y * side) + delta;
                    edge_peak = edge_peak.max(project(exact).distance(project(other)));
                    edge_peak =
                        edge_peak.max(project(exact).distance(project((start + end) * 0.5)));
                }
            }
        }
    }
    [peak, edge_peak]
}
