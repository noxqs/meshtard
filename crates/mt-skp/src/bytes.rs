//! Little-endian readers over byte slices. Out-of-range reads return `Err`, never panic.
//! Port of OpenSKP `tlv.cpp` (read helpers).

use crate::Error;

fn slice(d: &[u8], o: usize, n: usize) -> Result<&[u8], Error> {
    d.get(o..o.checked_add(n).ok_or(Error::Truncated)?)
        .ok_or(Error::Truncated)
}

pub fn u16_at(d: &[u8], o: usize) -> Result<u16, Error> {
    Ok(u16::from_le_bytes(slice(d, o, 2)?.try_into().unwrap()))
}

pub fn u32_at(d: &[u8], o: usize) -> Result<u32, Error> {
    Ok(u32::from_le_bytes(slice(d, o, 4)?.try_into().unwrap()))
}

pub fn i32_at(d: &[u8], o: usize) -> Result<i32, Error> {
    Ok(u32_at(d, o)? as i32)
}

pub fn f64_at(d: &[u8], o: usize) -> Result<f64, Error> {
    Ok(f64::from_le_bytes(slice(d, o, 8)?.try_into().unwrap()))
}

pub fn vec3_at(d: &[u8], o: usize) -> Result<[f64; 3], Error> {
    Ok([f64_at(d, o)?, f64_at(d, o + 8)?, f64_at(d, o + 16)?])
}

/// Little-endian unsigned integer of `n` bytes (at most 8).
pub fn varint(d: &[u8], o: usize, n: usize) -> Result<u64, Error> {
    if n > 8 {
        return Err(Error::Malformed("varint exceeds 64 bits".into()));
    }
    Ok(slice(d, o, n)?
        .iter()
        .enumerate()
        .fold(0u64, |v, (i, b)| v | u64::from(*b) << (8 * i)))
}

/// The whole slice as a little-endian unsigned integer.
pub fn varint_all(d: &[u8]) -> Result<u64, Error> {
    varint(d, 0, d.len())
}

/// A 2-byte tag as four uppercase hex digits in byte order, e.g. bytes `C4 09` -> `"C409"`.
pub fn tag_at(d: &[u8], o: usize) -> Result<Tag, Error> {
    let b = slice(d, o, 2)?;
    Ok(Tag([b[0], b[1]]))
}

pub fn hex_upper(d: &[u8]) -> String {
    use std::fmt::Write;
    d.iter()
        .fold(String::with_capacity(d.len() * 2), |mut s, b| {
            let _ = write!(s, "{b:02X}");
            s
        })
}

pub fn text(d: &[u8]) -> String {
    String::from_utf8_lossy(d).into_owned()
}

/// A TLV tag. Compared against string literals in OpenSKP's byte-order hex form.
#[derive(Clone, Copy, Default, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Tag(pub [u8; 2]);

impl Tag {
    /// Parses `"C409"` into bytes `C4 09`. Panics on malformed literals (programmer error).
    pub const fn new(s: &str) -> Self {
        const fn nib(c: u8) -> u8 {
            match c {
                b'0'..=b'9' => c - b'0',
                b'A'..=b'F' => c - b'A' + 10,
                _ => panic!("tag literal must be uppercase hex"),
            }
        }
        let b = s.as_bytes();
        assert!(b.len() == 4, "tag literal must have 4 hex digits");
        Tag([nib(b[0]) << 4 | nib(b[1]), nib(b[2]) << 4 | nib(b[3])])
    }
}

impl std::fmt::Debug for Tag {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:02X}{:02X}", self.0[0], self.0[1])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tag_literal_matches_bytes() {
        assert_eq!(Tag::new("C409"), tag_at(&[0xC4, 0x09], 0).unwrap());
        assert_eq!(format!("{:?}", Tag::new("DE05")), "DE05");
    }

    #[test]
    fn reads_are_bounds_checked() {
        assert!(u32_at(&[1, 2, 3], 0).is_err());
        assert!(f64_at(&[0; 8], 1).is_err());
        assert_eq!(varint(&[0x34, 0x12], 0, 2).unwrap(), 0x1234);
        assert!(varint(&[0; 9], 0, 9).is_err());
    }
}
