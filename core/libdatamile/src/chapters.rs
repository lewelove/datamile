use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Chapter {
    pub title: String,
    pub start_ms: u64,
    pub end_ms: u64,
}

const ID3_MAGIC: &[u8] = b"ID3";
const CHAP_FRAME: &[u8] = b"CHAP";
const TIT2_FRAME: &[u8] = b"TIT2";
const CHPL_ATOM: &[u8] = b"chpl";

/// Extract container chapter markers, if any.
#[must_use]
pub fn extract(path: &Path) -> Option<Vec<Chapter>> {
    let data = std::fs::read(path).ok()?;
    let mut chapters =
        parse_id3v2_chapters(&data).or_else(|| parse_mp4_chapters(&data))?;

    chapters.retain(|chapter| chapter.end_ms >= chapter.start_ms);
    chapters.sort_by_key(|chapter| chapter.start_ms);

    (!chapters.is_empty()).then_some(chapters)
}

fn synchsafe(bytes: &[u8]) -> usize {
    bytes
        .iter()
        .take(4)
        .fold(0usize, |acc, byte| (acc << 7) | usize::from(byte & 0x7f))
}

fn u32_be(bytes: &[u8]) -> u32 {
    bytes
        .get(..4)
        .map_or(0, |b| u32::from_be_bytes([b[0], b[1], b[2], b[3]]))
}

fn u64_be(bytes: &[u8]) -> u64 {
    bytes.get(..8).map_or(0, |b| {
        u64::from_be_bytes([b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7]])
    })
}

fn deunsync(data: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(data.len());
    let mut i = 0;
    while i < data.len() {
        out.push(data[i]);
        if data[i] == 0xff && data.get(i + 1) == Some(&0) {
            i += 1;
        }
        i += 1;
    }
    out
}

fn latin1(bytes: &[u8]) -> String {
    bytes.iter().map(|&b| char::from(b)).collect()
}

#[derive(Clone, Copy)]
enum Endian {
    Little,
    Big,
}

fn utf16(bytes: &[u8], endian: Endian) -> String {
    let mut units = Vec::with_capacity(bytes.len() / 2);
    for pair in bytes.chunks(2) {
        if let [a, b] = pair {
            let unit = match endian {
                Endian::Little => u16::from_le_bytes([*a, *b]),
                Endian::Big => u16::from_be_bytes([*a, *b]),
            };
            units.push(unit);
        }
    }

    String::from_utf16_lossy(&units)
}

fn decode_text(bytes: &[u8]) -> Option<String> {
    let (&encoding, body) = bytes.split_first()?;
    let text = match encoding {
        0 => latin1(body),
        1 => {
            if body.starts_with(&[0xff, 0xfe]) {
                utf16(&body[2..], Endian::Little)
            } else if body.starts_with(&[0xfe, 0xff]) {
                utf16(&body[2..], Endian::Big)
            } else {
                utf16(body, Endian::Little)
            }
        }
        2 => utf16(body, Endian::Big),
        3 => String::from_utf8_lossy(body).into_owned(),
        _ => return None,
    };

    Some(text.trim_end_matches('\0').to_string())
}

fn frame_size(major: u8, bytes: &[u8]) -> usize {
    if major == 4 {
        synchsafe(bytes)
    } else {
        u32_be(bytes) as usize
    }
}

fn parse_chap_times(data: &[u8]) -> Option<(u64, u64, usize)> {
    let id_end = data.iter().position(|&b| b == 0)?;
    let rest = data.get(id_end + 1..)?;
    if rest.len() < 16 {
        return None;
    }

    let start_ms = u64::from(u32_be(&rest[0..4]));
    let end_ms = u64::from(u32_be(&rest[4..8]));

    Some((start_ms, end_ms, id_end + 17))
}

fn parse_chap(data: &[u8], major: u8) -> Option<Chapter> {
    let id_end = data.iter().position(|&b| b == 0)?;
    let id = latin1(&data[..id_end]);
    let (start_ms, end_ms, mut pos) = parse_chap_times(data)?;
    let mut title = None;

    while pos + 10 <= data.len() {
        let frame_id = &data[pos..pos + 4];
        if frame_id[0] == 0 {
            break;
        }
        let size = frame_size(major, &data[pos + 4..pos + 8]);
        if size == 0 || pos + 10 + size > data.len() {
            break;
        }
        if frame_id == TIT2_FRAME {
            title = decode_text(&data[pos + 10..pos + 10 + size]);
        }
        pos += 10 + size;
    }

    Some(Chapter {
        title: title.filter(|t| !t.is_empty()).unwrap_or(id),
        start_ms,
        end_ms,
    })
}

/// Parse `ID3v2` CHAP frames. Unsupported or malformed tags yield `None`.
#[must_use]
pub fn parse_id3v2_chapters(data: &[u8]) -> Option<Vec<Chapter>> {
    if data.get(..3)? != ID3_MAGIC || data.len() < 10 {
        return None;
    }

    let major = data[3];
    if major < 3 {
        return None;
    }

    let flags = data[5];
    let size = synchsafe(&data[6..10]);
    let end = (10 + size).min(data.len());
    let raw = &data[10..end];
    let body = if flags & 0x80 != 0 {
        deunsync(raw)
    } else {
        raw.to_vec()
    };

    let mut pos = if flags & 0x40 != 0 {
        if body.len() < 4 {
            return None;
        }
        if major == 4 {
            synchsafe(&body[..4])
        } else {
            u32_be(&body[..4]) as usize + 4
        }
    } else {
        0usize
    };

    let mut chapters = Vec::new();
    while pos + 10 <= body.len() {
        let frame_id = &body[pos..pos + 4];
        if frame_id[0] == 0 {
            break;
        }
        let size = frame_size(major, &body[pos + 4..pos + 8]);
        if size == 0 || pos + 10 + size > body.len() {
            break;
        }
        if frame_id == CHAP_FRAME
            && let Some(chapter) = parse_chap(&body[pos + 10..pos + 10 + size], major)
        {
            chapters.push(chapter);
        }
        pos += 10 + size;
    }

    (!chapters.is_empty()).then_some(chapters)
}

fn atom_type(data: &[u8], pos: usize) -> Option<&[u8]> {
    data.get(pos + 4..pos + 8)
}

fn find_chpl(data: &[u8], depth: usize) -> Option<&[u8]> {
    if depth > 8 {
        return None;
    }

    let mut pos = 0usize;
    while pos + 8 <= data.len() {
        let size =
            u32::from_be_bytes([data[pos], data[pos + 1], data[pos + 2], data[pos + 3]])
                as usize;
        let kind = atom_type(data, pos)?;
        let header = if size == 1 { 16 } else { 8 };
        let end = if size == 0 || size == 1 {
            data.len()
        } else {
            (pos + size).min(data.len())
        };
        if end <= pos + header {
            break;
        }

        let payload = &data[pos + header..end];
        if kind == CHPL_ATOM {
            return Some(payload);
        }

        let container = matches!(
            kind,
            b"moov" | b"trak" | b"mdia" | b"minf" | b"stbl" | b"udta" | b"meta"
        );
        if container {
            let nested = if kind == b"meta" && payload.len() >= 4 {
                &payload[4..]
            } else {
                payload
            };
            if let Some(found) = find_chpl(nested, depth + 1) {
                return Some(found);
            }
        }

        if size == 0 {
            break;
        }
        pos = end;
    }

    None
}

fn parse_chpl_entries(data: &[u8], offset: usize) -> Option<Vec<Chapter>> {
    let count = usize::from(*data.get(offset)?);
    let mut pos = offset + 1;
    let mut chapters = Vec::with_capacity(count);

    for _ in 0..count {
        let start = u64_be(data.get(pos..pos + 8)?) / 10_000;
        let len = usize::from(*data.get(pos + 8)?);
        let title = data
            .get(pos + 9..pos + 9 + len)
            .map(|b| String::from_utf8_lossy(b).into_owned())?;
        chapters.push(Chapter {
            title,
            start_ms: start,
            end_ms: start,
        });
        pos += 9 + len;
    }

    for i in 0..chapters.len() {
        let end = chapters
            .get(i + 1)
            .map_or(chapters[i].start_ms, |next| next.start_ms);
        chapters[i].end_ms = end;
    }

    (!chapters.is_empty()).then_some(chapters)
}

/// Parse `QuickTime` `chpl` chapters. Accepts the common reserved-field layouts.
#[must_use]
pub fn parse_mp4_chapters(data: &[u8]) -> Option<Vec<Chapter>> {
    let payload = find_chpl(data, 0)?;

    for offset in [8usize, 4usize, 5usize] {
        if let Some(chapters) = parse_chpl_entries(payload, offset) {
            return Some(chapters);
        }
    }

    None
}

#[cfg(test)]
mod tests;
