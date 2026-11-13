use super::*;
use std::path::{Path, PathBuf};

fn atom(kind: &[u8], payload: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    let size = u32::try_from(payload.len() + 8).unwrap();
    out.extend_from_slice(&size.to_be_bytes());
    out.extend_from_slice(kind);
    out.extend_from_slice(payload);
    out
}

fn id3_frame(id: &[u8], body: &[u8]) -> Vec<u8> {
    id3_frame_flags(id, body, [0, 0])
}

fn id3_frame_flags(id: &[u8], body: &[u8], flags: [u8; 2]) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(id);
    out.extend_from_slice(&u32::try_from(body.len()).unwrap().to_be_bytes());
    out.extend_from_slice(&flags);
    out.extend_from_slice(body);
    out
}

fn id3_tag(body: &[u8]) -> Vec<u8> {
    id3_tag_major(3, body)
}

fn id3_tag_major(major: u8, body: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(ID3_MAGIC);
    out.extend_from_slice(&[major, 0, 0]);
    let size = u32::try_from(body.len()).unwrap();
    let synchsafe = [
        ((size >> 21) & 0x7f) as u8,
        ((size >> 14) & 0x7f) as u8,
        ((size >> 7) & 0x7f) as u8,
        (size & 0x7f) as u8,
    ];
    out.extend_from_slice(&synchsafe);
    out.extend_from_slice(body);
    out
}

fn chap_body(id: &str, start: u32, end: u32, title: &str) -> Vec<u8> {
    chap_body_bytes(id, start, end, 3, title.as_bytes())
}

fn chap_body_bytes(
    id: &str,
    start: u32,
    end: u32,
    encoding: u8,
    title: &[u8],
) -> Vec<u8> {
    let mut body = Vec::new();
    body.extend_from_slice(id.as_bytes());
    body.push(0);
    body.extend_from_slice(&start.to_be_bytes());
    body.extend_from_slice(&end.to_be_bytes());
    body.extend_from_slice(&u32::MAX.to_be_bytes());
    body.extend_from_slice(&u32::MAX.to_be_bytes());
    let mut text = vec![encoding];
    text.extend_from_slice(title);
    body.extend_from_slice(&id3_frame(TIT2_FRAME, &text));
    body
}

fn write_file(dir: &Path, name: &str, bytes: &[u8]) -> PathBuf {
    let path = dir.join(name);
    std::fs::write(&path, bytes).expect("write fixture");
    path
}

fn chpl_payload(version: u8) -> Vec<u8> {
    let mut chpl = Vec::new();
    chpl.extend_from_slice(&[version, 0, 0, 0]);
    if version == 1 {
        chpl.extend_from_slice(&[0, 0, 0, 0]);
    }
    chpl.push(2);
    for (start, title) in [(0u64, "Intro"), (600_000_000, "Main")] {
        chpl.extend_from_slice(&start.to_be_bytes());
        chpl.push(u8::try_from(title.len()).unwrap());
        chpl.extend_from_slice(title.as_bytes());
    }
    chpl
}

fn moov_with_chpl() -> Vec<u8> {
    atom(b"moov", &atom(CHPL_ATOM, &chpl_payload(0)))
}

/// ID3v2.3 CHAP frames map into sorted chapter markers.
#[test]
fn parses_id3v23_chapters() {
    let mut body = Vec::new();
    body.extend_from_slice(&id3_frame(
        CHAP_FRAME,
        &chap_body("ch1", 0, 60_000, "Intro"),
    ));
    body.extend_from_slice(&id3_frame(
        CHAP_FRAME,
        &chap_body("ch2", 60_000, 120_000, "Main"),
    ));
    let tag = id3_tag(&body);

    let tmp = tempfile::tempdir().expect("tempdir");
    let path = write_file(tmp.path(), "track.mp3", &tag);

    let chapters = extract(&path).expect("chapters");
    assert_eq!(
        chapters,
        vec![
            Chapter {
                title: "Intro".to_string(),
                start_ms: 0,
                end_ms: 60_000,
            },
            Chapter {
                title: "Main".to_string(),
                start_ms: 60_000,
                end_ms: 120_000,
            },
        ]
    );
}

/// Archived `mdat` boxes may precede `moov`; the walker must honor 64-bit sizes.
#[test]
fn extended_mdat_before_moov() {
    let mut mp4 = atom(b"ftyp", b"isom");
    let mut mdat = Vec::new();
    mdat.extend_from_slice(&1u32.to_be_bytes());
    mdat.extend_from_slice(b"mdat");
    mdat.extend_from_slice(&24u64.to_be_bytes());
    mdat.extend_from_slice(b"garbage!");
    mp4.extend_from_slice(&mdat);
    mp4.extend_from_slice(&moov_with_chpl());

    let tmp = tempfile::tempdir().expect("tempdir");
    let path = write_file(tmp.path(), "book.m4b", &mp4);

    let chapters = extract(&path).expect("chapters");
    assert_eq!(chapters[0].title, "Intro");
}

/// MP4 `chpl` atoms map into chapter markers with derived end times.
#[test]
fn parses_mp4_chpl_chapters() {
    let mut mp4 = atom(b"ftyp", b"isom");
    mp4.extend_from_slice(&moov_with_chpl());

    let tmp = tempfile::tempdir().expect("tempdir");
    let path = write_file(tmp.path(), "track.m4a", &mp4);

    let chapters = extract(&path).expect("chapters");
    assert_eq!(chapters.len(), 2);
    assert_eq!(chapters[0].title, "Intro");
    assert_eq!(chapters[1].start_ms, 60_000);
}

/// Version 1 `chpl` boxes carry an extra reserved field.
#[test]
fn parses_v1_chpl() {
    let mut mp4 = atom(b"ftyp", b"isom");
    mp4.extend_from_slice(&atom(b"moov", &atom(CHPL_ATOM, &chpl_payload(1))));

    let tmp = tempfile::tempdir().expect("tempdir");
    let path = write_file(tmp.path(), "track.m4a", &mp4);

    let chapters = extract(&path).expect("chapters");
    assert_eq!(chapters[0].title, "Intro");
}

/// The final `chpl` chapter inherits the track duration.
#[test]
fn closes_last_chapter() {
    let chapters = vec![
        Chapter {
            title: "Intro".to_string(),
            start_ms: 0,
            end_ms: 60_000,
        },
        Chapter {
            title: "Main".to_string(),
            start_ms: 60_000,
            end_ms: 60_000,
        },
    ];

    let closed = close_last_chapter(chapters, 120_000);
    assert_eq!(closed[1].end_ms, 120_000);
}

/// ID3v2.4 frame-level unsync applies inside `CHAP` frames.
#[test]
fn parses_v24_frame_unsync() {
    let mut body = chap_body_bytes("ch1", 0, 1_000, 0, b"A\xff\x00B");
    let size_at = "ch1".len() + 1 + 16 + 4;
    body[size_at..size_at + 4].copy_from_slice(&4u32.to_be_bytes());

    let frame = id3_frame_flags(CHAP_FRAME, &body, [0, 0x02]);
    let tag = id3_tag_major(4, &frame);

    let tmp = tempfile::tempdir().expect("tempdir");
    let path = write_file(tmp.path(), "track.mp3", &tag);

    let chapters = extract(&path).expect("chapters");
    assert_eq!(chapters[0].title, "A\u{ff}B");
}
