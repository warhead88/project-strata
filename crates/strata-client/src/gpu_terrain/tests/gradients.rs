use strata_terrain::noise::gradient_index;

use super::{
    Buffer, BufferUsages, CommandEncoderDescriptor, RenderDevice, RenderQueue, allocate, read,
};

pub(super) fn validate(
    device: &RenderDevice,
    queue: &RenderQueue,
    octaves: [(u32, u64); 2],
    lattice: &Buffer,
    second_octave_offset: u32,
) -> Result<(), Box<dyn std::error::Error>> {
    let size = lattice.size();
    let readback = allocate(
        device,
        "Terrain lattice parity readback",
        size,
        BufferUsages::MAP_READ | BufferUsages::COPY_DST,
    );
    let mut encoder = device.create_command_encoder(&CommandEncoderDescriptor::default());
    encoder.copy_buffer_to_buffer(lattice, 0, &readback, 0, size);
    queue.submit([encoder.finish()]);
    let words = read::<u32>(device, &readback)?;
    let [first, second] = octaves;
    let split = second_octave_offset as usize;
    let (first_words, second_words) = words.split_at(split);
    check_octave(first_words, first)?;
    check_octave(second_words, second)?;
    println!(
        "Matched {} gradient indices bit for bit",
        octaves.iter().map(|(f, _)| (2 * f + 1).pow(3)).sum::<u32>()
    );
    Ok(())
}

fn check_octave(
    words: &[u32],
    (frequency, seed): (u32, u64),
) -> Result<(), std::num::TryFromIntError> {
    let freq = i32::try_from(frequency)?;
    let count = usize::try_from((frequency * 2 + 1).pow(3))?;
    assert_eq!(words.len(), count.div_ceil(8));
    let mut index = 0usize;
    for z in -freq..=freq {
        for y in -freq..=freq {
            for x in -freq..=freq {
                let actual = (words[index / 8] >> ((index % 8) * 4)) & 15;
                let expected = gradient_index(x, y, z, seed);
                assert_eq!(
                    actual, expected,
                    "seed {seed:#018x}, coordinate {x},{y},{z}"
                );
                index += 1;
            }
        }
    }
    if !count.is_multiple_of(8) {
        assert_eq!(words[count / 8] >> ((count % 8) * 4), 0);
    }
    Ok(())
}
