use memchr::memmem;

use crate::error::{Error, Result};

/// A bounds-checked, zero-copy reader over decompressed replay bytes.
#[derive(Clone, Copy)]
pub struct Cursor<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> Cursor<'a> {
    pub fn new(data: &'a [u8], pos: usize) -> Self {
        Self { data, pos }
    }

    pub fn pos(&self) -> usize {
        self.pos
    }

    /// Up to `n` bytes ending at the current position.
    pub fn behind(&self, n: usize) -> &'a [u8] {
        &self.data[self.pos.saturating_sub(n)..self.pos]
    }

    /// Up to `n` bytes from the current position, without advancing.
    pub fn peek(&self, n: usize) -> &'a [u8] {
        &self.data[self.pos..self.data.len().min(self.pos.saturating_add(n))]
    }

    pub fn skip(&mut self, n: usize) -> Result<()> {
        self.bytes(n).map(|_| ())
    }

    pub fn bytes(&mut self, n: usize) -> Result<&'a [u8]> {
        let end = self.pos.checked_add(n).ok_or(Error::UnexpectedEof)?;
        let out = self.data.get(self.pos..end).ok_or(Error::UnexpectedEof)?;
        self.pos = end;
        Ok(out)
    }

    pub fn array<const N: usize>(&mut self) -> Result<[u8; N]> {
        Ok(self.bytes(N)?.try_into().expect("slice has length N"))
    }

    pub fn u8(&mut self) -> Result<u8> {
        Ok(self.array::<1>()?[0])
    }

    /// A string prefixed by a single length byte. Invalid UTF-8 is replaced
    /// rather than rejected; usernames are user-controlled.
    pub fn string(&mut self) -> Result<String> {
        let len = self.u8()? as usize;
        Ok(String::from_utf8_lossy(self.bytes(len)?).into_owned())
    }

    /// A little-endian u32 preceded by its (always 4) size byte.
    pub fn u32(&mut self) -> Result<u32> {
        self.skip(1)?;
        Ok(u32::from_le_bytes(self.array()?))
    }

    /// A little-endian u64 preceded by its (always 8) size byte.
    pub fn u64(&mut self) -> Result<u64> {
        self.skip(1)?;
        Ok(u64::from_le_bytes(self.array()?))
    }

    /// Moves to just past the next occurrence of `pattern`.
    pub fn seek(&mut self, pattern: &[u8]) -> Result<()> {
        let rest = &self.data[self.pos..];
        let found = memmem::find(rest, pattern).ok_or(Error::UnexpectedEof)?;
        self.pos += found + pattern.len();
        Ok(())
    }
}
