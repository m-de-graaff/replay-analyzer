//! Turns a raw `.rec` file into the decompressed dissect byte stream.
//!
//! Two layouts exist:
//! - Before Y8S4 the whole file is a zstd stream whose output starts with the
//!   `dissect` header.
//! - From Y8S4 the file starts with a plain `dissect` header followed by many
//!   independent zstd frames. Those frames are decompressed in parallel.

use std::io::Read;

use memchr::memmem;
use rayon::prelude::*;
use zstd::zstd_safe;

use crate::container::{self, Container};
use crate::error::{Error, Result};
use crate::format::{self, FormatInfo, FrameIndex, Layout};
use crate::header;

const ZSTD_MAGIC: [u8; 4] = [0x28, 0xB5, 0x2F, 0xFD];
const DISSECT_MAGIC: &[u8] = b"diss";

/// The decompressed replay together with where the header's properties end.
pub struct Decompressed {
    pub data: Vec<u8>,
    pub header: header::Header,
    /// Offset in `data` where packet data begins.
    pub body_start: usize,
    pub format: FormatInfo,
    /// Number of zstd frames in the file.
    pub zstd_frames: usize,
    /// Frame timestamps, when the index was found.
    pub frame_index: Option<FrameIndex>,
    /// Streams and blocks of a Y8S4+ file.
    pub container: Option<Container>,
}

pub fn decompress(raw: &[u8]) -> Result<Decompressed> {
    if raw.starts_with(&ZSTD_MAGIC) {
        let frames = locate_frames(raw, 0);
        let data = decompress_frames(raw, &frames)?;
        let (header, mut format, body_start) = header::parse(&data)?;
        format.layout = Layout::Stream;
        let frame_index = data
            .get(body_start..)
            .and_then(|b| format::read_frame_index(b, format.declared_frames));
        Ok(Decompressed {
            data,
            header,
            body_start,
            format,
            zstd_frames: frames.len(),
            frame_index,
            container: None,
        })
    } else if raw.starts_with(DISSECT_MAGIC) {
        let (header, mut format, header_end) = header::parse(raw)?;
        format.layout = Layout::Chunked;
        let (frame_index, mut container) =
            chunked_container(raw, header_end, format.declared_frames);
        // A complete map knows exactly where every block is. Otherwise fall
        // back to finding zstd frames by their magic, as before the container
        // was understood.
        let frames = match &container {
            Some(c) if c.complete => c.frames.clone(),
            Some(_) => {
                // The walk found the file ends early. A last frame the end
                // cuts off is left out, so the rest still reads.
                let mut frames = locate_frames(raw, header_end);
                if frames.last().is_some_and(|&(from, _)| {
                    zstd_safe::find_frame_compressed_size(&raw[from..]).is_err()
                }) {
                    frames.pop();
                }
                frames
            }
            None => locate_frames(raw, header_end),
        };
        if let Some(c) = container
            .as_mut()
            .filter(|c| !c.complete && c.frames.len() != frames.len())
        {
            c.warnings.push(format!(
                "{} zstd frames found by scanning, {} by walking the blocks",
                frames.len(),
                c.frames.len()
            ));
        }
        let data = decompress_frames(raw, &frames)?;
        Ok(Decompressed {
            data,
            header,
            body_start: 0,
            format,
            zstd_frames: frames.len(),
            frame_index,
            container,
        })
    } else {
        Err(Error::InvalidFile)
    }
}

/// The uncompressed frame index after the header of a Y8S4+ file, and the
/// container after it.
fn chunked_container(
    raw: &[u8],
    header_end: usize,
    declared_frames: u32,
) -> (Option<FrameIndex>, Option<Container>) {
    // Without a frame count from the prelude, only where the compressed data
    // starts bounds the index.
    let end = match declared_frames {
        0 => raw
            .get(header_end..)
            .and_then(|rest| memmem::find(rest, &ZSTD_MAGIC))
            .map_or(raw.len(), |i| header_end + i),
        _ => raw.len(),
    };
    let frame_index = raw
        .get(header_end..end)
        .and_then(|b| format::read_frame_index(b, declared_frames));
    let container = frame_index
        .as_ref()
        .map(|i| container::map(raw, header_end + i.len));
    (frame_index, container)
}

/// Like [`decompress`], but for Y8S4+ replays only the uncompressed header and
/// frame index are read and `data` is left empty.
pub fn header_only(raw: &[u8]) -> Result<Decompressed> {
    if !raw.starts_with(DISSECT_MAGIC) {
        return decompress(raw);
    }
    let (header, mut format, header_end) = header::parse(raw)?;
    format.layout = Layout::Chunked;
    let (frame_index, container) = chunked_container(raw, header_end, format.declared_frames);
    Ok(Decompressed {
        data: Vec::new(),
        header,
        body_start: 0,
        format,
        zstd_frames: 0,
        frame_index,
        container,
    })
}

/// Decompresses the given zstd frames and concatenates the output. Non-zstd
/// bytes between or after frames are ignored, matching the original tool
/// which tolerated non-zstd trailers. A damaged frame is an error.
fn decompress_frames(raw: &[u8], frames: &[(usize, usize)]) -> Result<Vec<u8>> {
    if frames.is_empty() {
        return Err(Error::InvalidFile);
    }
    tracing::debug!(frames = frames.len(), "decompressing zstd frames");
    let chunks = frames
        .par_iter()
        .map(|&(from, to)| decompress_frame(&raw[from..to]))
        .collect::<Vec<_>>();
    let chunks = chunks.into_iter().collect::<Result<Vec<_>>>()?;
    Ok(chunks.concat())
}

/// Finds `(start, end)` byte ranges of consecutive zstd frames.
fn locate_frames(raw: &[u8], mut pos: usize) -> Vec<(usize, usize)> {
    let finder = memmem::Finder::new(&ZSTD_MAGIC);
    let mut frames = Vec::new();
    while let Some(found) = finder.find(&raw[pos..]) {
        let from = pos + found;
        match zstd_safe::find_frame_compressed_size(&raw[from..]) {
            Ok(size) => {
                frames.push((from, from + size));
                pos = from + size;
            }
            Err(_) => {
                // Truncated or damaged: the decoder reports the error.
                frames.push((from, raw.len()));
                break;
            }
        }
    }
    frames
}

fn decompress_frame(frame: &[u8]) -> Result<Vec<u8>> {
    let hint = match zstd_safe::get_frame_content_size(frame) {
        Ok(Some(n)) => n as usize,
        _ => frame.len() * 4,
    };
    let mut out = Vec::with_capacity(hint);
    let mut decoder = zstd::stream::read::Decoder::with_buffer(frame)
        .map_err(Error::Decompress)?
        .single_frame();
    decoder.read_to_end(&mut out).map_err(Error::Decompress)?;
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An index claiming `claimed` frames but holding `real`, then a zstd
    /// frame and zeros enough to look like more entries.
    fn file_with_index(claimed: u32, real: u32) -> Vec<u8> {
        let mut raw = b"header".to_vec();
        raw.extend([0; 8]);
        raw.extend(claimed.to_le_bytes());
        for i in 0..real {
            raw.extend(i.to_le_bytes());
            raw.extend((f64::from(i) / 30.0).to_le_bytes());
        }
        raw.extend(zstd::bulk::compress(b"body", 1).unwrap());
        raw.extend(vec![0; 20_000]);
        raw
    }

    #[test]
    fn without_a_frame_count_the_index_ends_where_compressed_data_starts() {
        // 1000 entries do not fit before the zstd frame, so this is no index.
        let raw = file_with_index(1000, 3);
        assert!(chunked_container(&raw, 6, 0).0.is_none());
        let raw = file_with_index(3, 3);
        assert_eq!(chunked_container(&raw, 6, 0).0.unwrap().times.len(), 3);
    }
}
