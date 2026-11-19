use std::collections::HashMap;

use bevy::camera::CameraProjection;

use strata_globe::{TileEdge, TileId};

use super::{super::source::vec4_array, *};

pub(super) fn validate(
    device: &RenderDevice,
    queue: &RenderQueue,
    input: &ExtractedTerrain,
    terrain: &TerrainGpu,
) -> Result<(), Box<dyn std::error::Error>> {
    let tile_bytes = TILE_CAPACITY * 16;
    let readback = allocate(
        device,
        "Terrain selection readback",
        2 * DRAW_BYTES + tile_bytes,
        BufferUsages::COPY_DST | BufferUsages::MAP_READ,
    );
    let mut frame = input.frame(Mat4::ZERO, 623.0, terrain.source);
    frame.planet[1] = frame.planet[2]; // no horizon cull, validate coverage of the complete sphere
    queue.write_buffer(&terrain.frame, 0, bytes_of(&frame));
    let mut encoder = device.create_command_encoder(&CommandEncoderDescriptor::default());
    terrain.encode_selection(&mut encoder, frame.settings[0], None);
    encoder.copy_buffer_to_buffer(&terrain.indirect, 0, &readback, 0, 2 * DRAW_BYTES);
    encoder.copy_buffer_to_buffer(&terrain.land, 0, &readback, 2 * DRAW_BYTES, tile_bytes);
    queue.submit([encoder.finish()]);
    let data = read::<u32>(device, &readback)?;
    let count = data[1] as usize;
    assert!(count > 6 * 256);
    assert!(data[6] > 0 && data[6] <= data[1]);
    let mut leaves = HashMap::with_capacity(count);
    let mut area = [0; 6];
    let max_lod = input.camera.planet.max_lod();
    for raw in data[10..10 + count * 4].chunks_exact(4) {
        let face = CubeFace::ALL[(raw[3] & 7) as usize];
        let tile = TileId::new(face, u8::try_from(raw[2])?, raw[0], raw[1])?;
        area[face.index()] += 1u64 << (2 * (max_lod - tile.lod));
        assert!(leaves.insert(tile, raw[3] >> 3).is_none());
    }
    assert_eq!(area, [1u64 << (2 * max_lod); 6]);
    assert!(leaves.keys().any(|tile| tile.lod == max_lod));
    let mut cross_face_stitches = 0;
    for (&tile, &edges) in &leaves {
        let mut ancestor = tile.parent();
        while let Some(node) = ancestor {
            assert!(!leaves.contains_key(&node));
            ancestor = node.parent();
        }
        for (bit, edge) in [
            TileEdge::Left,
            TileEdge::Right,
            TileEdge::Bottom,
            TileEdge::Top,
        ]
        .into_iter()
        .enumerate()
        {
            let neighbor = tile.neighbor(edge);
            let mut ancestor = Some(neighbor.tile);
            let mut coarser = false;
            while let Some(node) = ancestor {
                if leaves.contains_key(&node) {
                    assert!(tile.lod - node.lod <= 1);
                    coarser = node.lod < tile.lod;
                    break;
                }
                ancestor = node.parent();
            }
            if neighbor.crossed_face && coarser {
                cross_face_stitches += 1;
            }
            assert_eq!((edges >> bit) & 1 != 0, coarser, "{tile:?} {edge:?}");
        }
    }
    assert!(cross_face_stitches > 0);
    println!(
        "GPU selected {count} balanced leaves and stitched {cross_face_stitches} cross-face LOD transitions"
    );
    let (eye, target, up) = input.camera.compute_pose_f64();
    let as_vec3 = |v| Vec4::from_array(vec4_array(v, 0.0)).truncate();
    let projection = PerspectiveProjection {
        fov: 60.0_f32.to_radians(),
        aspect_ratio: 16.0 / 9.0,
        near: 0.4,
        ..default()
    }
    .get_clip_from_view();
    let rotation = Mat4::from_quat(
        Transform::IDENTITY
            .looking_at(as_vec3(target - eye), as_vec3(up))
            .rotation
            .inverse(),
    );
    let visible = input.frame(projection * rotation, 623.0, terrain.source);
    let visible_count = selected_counts(device, queue, terrain, &visible)?[0];
    assert!(visible_count > 0 && visible_count < data[1]);
    println!("Camera frustum and horizon retained {visible_count} of {count} leaves");
    Ok(())
}

fn selected_counts(
    device: &RenderDevice,
    queue: &RenderQueue,
    terrain: &TerrainGpu,
    frame: &GpuView,
) -> Result<[u32; 2], Box<dyn std::error::Error>> {
    let readback = allocate(
        device,
        "Terrain indirect count readback",
        2 * DRAW_BYTES,
        BufferUsages::COPY_DST | BufferUsages::MAP_READ,
    );
    queue.write_buffer(&terrain.frame, 0, bytes_of(frame));
    let mut encoder = device.create_command_encoder(&CommandEncoderDescriptor::default());
    terrain.encode_selection(&mut encoder, frame.settings[0], None);
    encoder.copy_buffer_to_buffer(&terrain.indirect, 0, &readback, 0, 2 * DRAW_BYTES);
    queue.submit([encoder.finish()]);
    let counts = read::<u32>(device, &readback)?;
    Ok([counts[1], counts[6]])
}
