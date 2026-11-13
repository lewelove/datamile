use serde::{Deserialize, Serialize};
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Chapter {
    pub title: String,
    pub start_ms: u64,
    pub end_ms: u64,
}

const ID3_MAGIC: &[u8] = b"ID3";
const MP4_MAGIC: &[u8] = b"ftyp";
const CHAP_FRAME: &[u8] = b"CHAP";
const TIT2_FRAME: &[u8] = b"TIT2";
const CHPL_ATOM: &[u8] = b"chpl";
const MAX_ID3_TAG_BYTES: usize = 64 * 1024 * 1024;
const MAX_CHPL_BYTES: usize = 16 * 1024 * 1024;
const MAX_MP4_DEPTH: usize = 8;

/// Extract container chapter markers without loading audio payloads.
#[must_use]
pub fn extract(path: &Path) -> Option<Vec<Chapter>> {
    let mut file = File::open(path).ok()?;
    let mut magic = [0u8; 12];
    let read = file.read(&mut magic).ok()?;

    let chapters = if read >= 3 && &magic[..3] == ID3_MAGIC {
        file.seek(SeekFrom::Start(0)).ok()?;
        read_id3_chapters(&mut file)?
    } else if read >= 8 && &magic[4..8] == MP4_MAGIC {
        let end = file.seek(SeekFrom::End(0)).ok()?;
        walk_mp4(&mut file, 0, end, 0)?
    } else {
        return None;
    };

    normalize(chapters)
}

fn normalize(mut chapters: Vec<Chapter>) -> Option<Vec<Chapter>> {
    chapters.retain(|chapter| chapter.end_ms >= chapter.start_ms);
    chapters.sort_by_key(|chapter| chapter.start_ms);

    (!chapters.is_empty()).then_some(chapters)
}

fn read_id3_chapters<R: Read>(reader: &mut R) -> Option<Vec<Chapter>> {
    let mut header = [0u8; 10];
    reader.read_exact(&mut header).ok()?;

    let size = synchsafe(&header[6..10]);
    if size > MAX_ID3_TAG_BYTES {
        return None;
    }

    let mut tag = Vec::with_capacity(header.len() + size);
    tag.extend_from_slice(&header);
    let mut body = vec![0u8; size];
    reader.read_exact(&mut body).ok()?;
    tag.extend_from_slice(&body);

    parse_id3v2_chapters(&tag)
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
        if frame_id == CHAP_FRAME {
            let raw = &body[pos + 10..pos + 10 + size];
            let data = if major == 4 && body[pos + 9] & 0x02 != 0 {
                deunsync(raw)
            } else {
                raw.to_vec()
            };
            if let Some(chapter) = parse_chap(&data, major) {
                chapters.push(chapter);
            }
        }
        pos += 10 + size;
    }

    (!chapters.is_empty()).then_some(chapters)
}

fn read_at<R: Read + Seek>(reader: &mut R, pos: u64, buf: &mut [u8]) -> Option<()> {
    reader.seek(SeekFrom::Start(pos)).ok()?;
    reader.read_exact(buf).ok()?;
    Some(())
}

const fn is_mp4_container(kind: &[u8]) -> bool {
    matches!(
        kind,
        b"moov" | b"trak" | b"mdia" | b"minf" | b"stbl" | b"udta" | b"meta"
    )
}

/// Walk top-level boxes by seeking past payloads. `mdat` is never read.
fn walk_mp4<R: Read + Seek>(
    reader: &mut R,
    start: u64,
    end: u64,
    depth: usize,
) -> Option<Vec<Chapter>> {
    if depth > MAX_MP4_DEPTH || start >= end {
        return None;
    }

    let mut pos = start;
    while pos + 8 <= end {
        let mut header = [0u8; 16];
        read_at(reader, pos, &mut header[..8])?;

        let size32 = u32::from_be_bytes([header[0], header[1], header[2], header[3]]);
        let kind = [header[4], header[5], header[6], header[7]];
        let (size, header_len) = match size32 {
            0 => (end - pos, 8),
            1 => {
                read_at(reader, pos + 8, &mut header[8..16])?;
                let size = u64::from_be_bytes([
                    header[8], header[9], header[10], header[11], header[12], header[13],
                    header[14], header[15],
                ]);
                (size, 16)
            }
            size => (u64::from(size), 8),
        };

        if size < header_len {
            return None;
        }
        let atom_end = pos.checked_add(size)?;
        if atom_end > end {
            return None;
        }
        let payload_start = pos + header_len;

        if kind.as_slice() == CHPL_ATOM {
            return read_chpl(reader, payload_start, atom_end);
        }

        if is_mp4_container(&kind) {
            let nested = if kind.as_slice() == b"meta" {
                payload_start.checked_add(4)?
            } else {
                payload_start
            };
            if nested < atom_end
                && let Some(chapters) = walk_mp4(reader, nested, atom_end, depth + 1)
            {
                return Some(chapters);
            }
        }

        pos = atom_end;
    }

    None
}

fn read_chpl<R: Read + Seek>(
    reader: &mut R,
    start: u64,
    end: u64,
) -> Option<Vec<Chapter>> {
    let len = usize::try_from(end.checked_sub(start)?).ok()?;
    if len > MAX_CHPL_BYTES {
        return None;
    }

    let mut payload = vec![0u8; len];
    read_at(reader, start, &mut payload)?;

    let offset = match payload.first() {
        Some(0) => 4,
        Some(1) => 8,
        _ => return None,
    };
    parse_chpl_entries(&payload, offset)
}

/// Give the last `chpl` chapter the file duration it cannot encode.
#[must_use]
pub(crate) fn close_last_chapter(
    mut chapters: Vec<Chapter>,
    duration_ms: u64,
) -> Vec<Chapter> {
    if let Some(last) = chapters.last_mut()
        && last.end_ms == last.start_ms
    {
        last.end_ms = duration_ms.max(last.start_ms);
    }

    chapters
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

#[cfg(test)]
mod tests;
