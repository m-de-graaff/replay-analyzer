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

use crate::error::{Error, Result};
use crate::header;

const ZSTD_MAGIC: [u8; 4] = [0x28, 0xB5, 0x2F, 0xFD];
const DISSECT_MAGIC: &[u8] = b"diss";

/// The decompressed replay together with where the header's properties end.
pub struct Decompressed {
    pub data: Vec<u8>,
    pub header: header::Header,
    /// Offset in `data` where packet data begins.
    pub body_start: usize,
}

pub fn decompress(raw: &[u8]) -> Result<Decompressed> {
    if raw.starts_with(&ZSTD_MAGIC) {
        let data = decompress_frames(raw, 0)?;
        let (header, body_start) = header::parse(&data)?;
        Ok(Decompressed {
            data,
            header,
            body_start,
        })
    } else if raw.starts_with(DISSECT_MAGIC) {
        let (header, header_end) = header::parse(raw)?;
        let data = decompress_frames(raw, header_end)?;
        Ok(Decompressed {
            data,
            header,
            body_start: 0,
        })
    } else {
        Err(Error::InvalidFile)
    }
}

/// Decompresses every zstd frame found at or after `start` and concatenates
/// the output. Non-zstd bytes between or after frames are ignored, matching
/// the original tool which tolerated non-zstd trailers. A damaged frame is an
/// error.
fn decompress_frames(raw: &[u8], start: usize) -> Result<Vec<u8>> {
    let frames = locate_frames(raw, start);
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
