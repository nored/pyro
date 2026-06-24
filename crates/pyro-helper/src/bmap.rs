//! Minimal parser for bmap (block map) files produced by `bmaptool`.
//!
//! A .bmap lists which blocks of the *uncompressed* image actually contain
//! data, so we can write/validate only those ranges and skip blank regions.

use std::fs;

#[derive(Debug)]
pub struct Bmap {
    pub block_size: u64,
    pub image_size: u64,
    /// Inclusive block ranges that contain data, in ascending order.
    pub ranges: Vec<(u64, u64)>,
}

fn tag<'a>(xml: &'a str, name: &str) -> Option<&'a str> {
    let open = format!("<{name}>");
    let close = format!("</{name}>");
    let start = xml.find(&open)? + open.len();
    let end = xml[start..].find(&close)? + start;
    Some(xml[start..end].trim())
}

pub fn parse(path: &str) -> Result<Bmap, String> {
    let xml = fs::read_to_string(path).map_err(|e| format!("read bmap: {e}"))?;
    let block_size: u64 = tag(&xml, "BlockSize")
        .and_then(|s| s.parse().ok())
        .ok_or("bmap missing BlockSize")?;
    if block_size == 0 {
        return Err("bmap BlockSize is zero".into());
    }
    let image_size: u64 = tag(&xml, "ImageSize")
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);

    // Each mapped region is a <Range ...> start-end </Range> (or a single block).
    let mut ranges = Vec::new();
    let mut rest = xml.as_str();
    while let Some(rel) = rest.find("<Range") {
        let after = &rest[rel..];
        let gt = match after.find('>') {
            Some(i) => i + 1,
            None => break,
        };
        let close = match after.find("</Range>") {
            Some(i) => i,
            None => break,
        };
        let body = after[gt..close].trim();
        if let Some((a, b)) = parse_range(body) {
            ranges.push((a, b));
        }
        rest = &after[close + "</Range>".len()..];
    }
    if ranges.is_empty() {
        return Err("bmap has no block ranges".into());
    }
    ranges.sort_unstable();
    Ok(Bmap {
        block_size,
        image_size,
        ranges,
    })
}

fn parse_range(s: &str) -> Option<(u64, u64)> {
    if let Some((a, b)) = s.split_once('-') {
        Some((a.trim().parse().ok()?, b.trim().parse().ok()?))
    } else {
        let n: u64 = s.trim().parse().ok()?;
        Some((n, n))
    }
}

impl Bmap {
    /// Length in bytes of `block` (the final block may be short).
    pub fn block_len(&self, block: u64) -> usize {
        if self.image_size == 0 {
            return self.block_size as usize;
        }
        let start = block * self.block_size;
        let remaining = self.image_size.saturating_sub(start);
        remaining.min(self.block_size) as usize
    }
}
