use std::io::Read;

use ruzstd::decoding::StreamingDecoder;

use crate::TerrainError;

pub(crate) const fn invalid(reason: &'static str) -> TerrainError {
    TerrainError::InvalidEarthDem { reason }
}

pub(crate) fn read_rows(
    packed: &[u8],
    row_bytes: usize,
    count: usize,
    mut read: impl FnMut(&[u8]),
) -> Result<(), TerrainError> {
    let mut decoder =
        StreamingDecoder::new(packed).map_err(|_| invalid("invalid Earth archive"))?;
    let mut row = vec![0; row_bytes];
    for _ in 0..count {
        decoder
            .read_exact(&mut row)
            .map_err(|_| invalid("truncated Earth archive"))?;
        read(&row);
    }
    if decoder
        .read(&mut [0])
        .map_err(|_| invalid("invalid Earth archive"))?
        != 0
    {
        return Err(invalid("trailing data in Earth archive"));
    }
    Ok(())
}

pub(crate) fn unpack(packed: &[u8], limit: u64) -> Result<Vec<u8>, TerrainError> {
    let mut bytes = Vec::new();
    StreamingDecoder::new(packed)
        .map_err(|_| invalid("invalid Earth archive"))?
        .take(limit + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| invalid("truncated Earth archive"))?;
    if bytes.len() as u64 > limit {
        return Err(invalid("Earth archive exceeds size limit"));
    }
    Ok(bytes)
}
