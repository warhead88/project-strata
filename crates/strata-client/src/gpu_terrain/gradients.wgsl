// SplitMix64 as pairs (low, high) of wrapping u32 limbs. Multiplication
// only needs the low 64 bits, and each 16-bit partial product fits a u32.
const X_MIX = vec2(0x7f4a7c15u, 0x9e3779b9u);
const Y_MIX = vec2(0x1ce4e5b9u, 0xbf58476du);
const Z_MIX = vec2(0x133111ebu, 0x94d049bbu);

fn add64(a: vec2<u32>, b: vec2<u32>) -> vec2<u32> {
    let low = a.x + b.x;
    return vec2(low, a.y + b.y + u32(low < a.x));
}

fn mul_high(a: u32, b: u32) -> u32 {
    let a0 = a & 65535u;
    let b0 = b & 65535u;
    let a1 = a >> 16u;
    let b1 = b >> 16u;
    let middle = a1 * b0 + ((a0 * b0) >> 16u);
    let carry = a0 * b1 + (middle & 65535u);
    return a1 * b1 + (middle >> 16u) + (carry >> 16u);
}

fn mul64(a: vec2<u32>, b: vec2<u32>) -> vec2<u32> {
    return vec2(a.x * b.x, mul_high(a.x, b.x) + a.y * b.x + a.x * b.y);
}

fn shr64(v: vec2<u32>, n: u32) -> vec2<u32> {
    return vec2((v.x >> n) | (v.y << (32u - n)), v.y >> n);
}

fn signed64(n: i32) -> vec2<u32> {
    return vec2(bitcast<u32>(n), select(0u, 0xffffffffu, n < 0));
}

fn lattice_seed(octave_seed: vec2<u32>, p: vec3<i32>) -> vec2<u32> {
    let xy = add64(mul64(signed64(p.x), X_MIX), mul64(signed64(p.y), Y_MIX));
    return add64(add64(octave_seed, xy), mul64(signed64(p.z), Z_MIX));
}

fn gradient_index(key: vec2<u32>) -> u32 {
    let h0 = mul64(key ^ shr64(key, 30u), Y_MIX);
    let h1 = mul64(h0 ^ shr64(h0, 27u), Z_MIX);
    return (h1 ^ shr64(h1, 31u)).x & 15u;
}

struct Octave {
    seed: vec2<u32>,
    frequency: u32,
    side: u32,
    count: u32,
    word_offset: u32,
    _padding: vec2<u32>,
}

@group(0) @binding(0) var<uniform> octaves: array<Octave, 2>;
@group(0) @binding(1) var<storage, read_write> lattice: array<u32>;

@compute @workgroup_size(64)
fn generate_gradients(
    @builtin(global_invocation_id) id: vec3<u32>,
    @builtin(num_workgroups) groups: vec3<u32>,
) {
    let word = id.x + id.y * groups.x * 64u;
    if word >= arrayLength(&lattice) { return; }

    let octave = octaves[u32(word >= octaves[1].word_offset)];
    let start = (word - octave.word_offset) * 8u;
    let row = start / octave.side;
    let freq = i32(octave.frequency);
    var p = vec3<i32>(i32(start % octave.side), i32(row % octave.side), i32(row / octave.side)) - freq;
    var key = lattice_seed(octave.seed, p);
    var packed = 0u;
    let count = min(8u, octave.count - start);
    for (var i = 0u; i < count; i++) {
        packed |= gradient_index(key) << (4u * i);
        p.x++;
        key = add64(key, X_MIX);
        if p.x > freq {
            p.x = -freq;
            p.y++;
            if p.y > freq {
                p.y = -freq;
                p.z++;
            }
            key = lattice_seed(octave.seed, p);
        }
    }
    lattice[word] = packed;
}
