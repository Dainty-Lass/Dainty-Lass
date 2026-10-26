//! Little-endian reads over a manifest's unpacked data. Every read is bounds-checked, and a count is refused when
//! the bytes left could not hold that many entries, so a hostile count can't make the parser allocate.

use super::{Guid, ManifestError};

pub(super) struct Reader<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    pub(super) fn new(data: &'a [u8]) -> Self {
        Self { data, pos: 0 }
    }

    pub(super) fn pos(&self) -> usize {
        self.pos
    }

    pub(super) fn remaining(&self) -> usize {
        self.data.len() - self.pos
    }

    /// One section: its `u32` size (counted from the size field itself), then a version byte. `read` sees only the
    /// section's bytes, and whatever it leaves unread is skipped, as a newer writer's extra fields would be.
    pub(super) fn section<T>(
        &mut self,
        name: &'static str,
        read: impl FnOnce(&mut Reader<'a>, u8) -> Result<T, ManifestError>,
    ) -> Result<T, ManifestError> {
        let start = self.pos;
        let size = self.u32()? as usize;
        let end = start
            .checked_add(size)
            .filter(|end| size >= 5 && *end <= self.data.len())
            .ok_or(ManifestError::SectionSize { section: name })?;
        let mut inner = Reader {
            data: &self.data[..end],
            pos: self.pos,
        };
        let version = inner.u8()?;
        let value = read(&mut inner, version)?;
        self.pos = end;
        Ok(value)
    }

    pub(super) fn bytes(&mut self, n: usize) -> Result<&'a [u8], ManifestError> {
        let end = self
            .pos
            .checked_add(n)
            .filter(|end| *end <= self.data.len())
            .ok_or(ManifestError::Truncated { offset: self.pos })?;
        let out = &self.data[self.pos..end];
        self.pos = end;
        Ok(out)
    }

    pub(super) fn array<const N: usize>(&mut self) -> Result<[u8; N], ManifestError> {
        let mut out = [0; N];
        out.copy_from_slice(self.bytes(N)?);
        Ok(out)
    }

    /// Moves to `to`, which must be at or after the current position and inside the data.
    pub(super) fn skip_to(&mut self, to: usize) -> Result<(), ManifestError> {
        if to < self.pos || to > self.data.len() {
            return Err(ManifestError::Truncated { offset: self.pos });
        }
        self.pos = to;
        Ok(())
    }

    pub(super) fn u8(&mut self) -> Result<u8, ManifestError> {
        Ok(self.array::<1>()?[0])
    }

    pub(super) fn u32(&mut self) -> Result<u32, ManifestError> {
        Ok(u32::from_le_bytes(self.array()?))
    }

    pub(super) fn i32(&mut self) -> Result<i32, ManifestError> {
        Ok(i32::from_le_bytes(self.array()?))
    }

    pub(super) fn u64(&mut self) -> Result<u64, ManifestError> {
        Ok(u64::from_le_bytes(self.array()?))
    }

    pub(super) fn i64(&mut self) -> Result<i64, ManifestError> {
        Ok(i64::from_le_bytes(self.array()?))
    }

    pub(super) fn guid(&mut self) -> Result<Guid, ManifestError> {
        Ok(Guid([self.u32()?, self.u32()?, self.u32()?, self.u32()?]))
    }

    /// A `u32` count of entries that each take at least `min_each` bytes.
    pub(super) fn count(&mut self, min_each: usize, what: &'static str) -> Result<usize, ManifestError> {
        let n = self.u32()? as usize;
        match n.checked_mul(min_each) {
            Some(bytes) if bytes <= self.remaining() => Ok(n),
            _ => Err(ManifestError::CountTooLarge { what }),
        }
    }

    /// Unreal's FString: an `i32` length counting the NUL; positive is 8-bit text, negative is UTF-16LE.
    pub(super) fn fstring(&mut self) -> Result<String, ManifestError> {
        let at = self.pos;
        let bad = ManifestError::BadString { offset: at };
        let len = self.i32()?;
        if len == 0 {
            return Ok(String::new());
        }
        let n = len.unsigned_abs() as usize;
        if len > 0 {
            let raw = self.bytes(n)?;
            match raw.split_last() {
                Some((0, text)) => String::from_utf8(text.to_vec()).map_err(|_| bad),
                _ => Err(bad),
            }
        } else {
            let raw = self.bytes(n.checked_mul(2).ok_or(ManifestError::Truncated { offset: at })?)?;
            let units: Vec<u16> = raw.as_chunks::<2>().0.iter().map(|b| u16::from_le_bytes(*b)).collect();
            match units.split_last() {
                Some((0, text)) => String::from_utf16(text).map_err(|_| bad),
                _ => Err(bad),
            }
        }
    }
}
