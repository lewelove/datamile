use super::*;

fn id3_frame(id: &[u8], body: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(id);
    out.extend_from_slice(&u32::try_from(body.len()).unwrap().to_be_bytes());
    out.extend_from_slice(&[0, 0]);
    out.extend_from_slice(body);
    out
}

fn id3_tag(body: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(ID3_MAGIC);
    out.extend_from_slice(&[3, 0, 0]);
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
    let mut body = Vec::new();
    body.extend_from_slice(id.as_bytes());
    body.push(0);
    body.extend_from_slice(&start.to_be_bytes());
    body.extend_from_slice(&end.to_be_bytes());
    body.extend_from_slice(&u32::MAX.to_be_bytes());
    body.extend_from_slice(&u32::MAX.to_be_bytes());
    let mut text = vec![3u8];
    text.extend_from_slice(title.as_bytes());
    body.extend_from_slice(&id3_frame(TIT2_FRAME, &text));
    body
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

    let chapters = parse_id3v2_chapters(&tag).expect("chapters");
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

/// MP4 `chpl` atoms map into chapter markers with derived end times.
#[test]
fn parses_mp4_chpl_chapters() {
    let mut chpl = Vec::new();
    chpl.extend_from_slice(&[0, 0, 0, 0]);
    chpl.extend_from_slice(&[0, 0, 0, 0]);
    chpl.push(2);
    for (start, title) in [(0u64, "Intro"), (600_000_000, "Main")] {
        chpl.extend_from_slice(&start.to_be_bytes());
        chpl.push(u8::try_from(title.len()).unwrap());
        chpl.extend_from_slice(title.as_bytes());
    }

    let mut atom = Vec::new();
    let size = u32::try_from(chpl.len() + 8).unwrap();
    atom.extend_from_slice(&size.to_be_bytes());
    atom.extend_from_slice(CHPL_ATOM);
    atom.extend_from_slice(&chpl);

    let mut moov = Vec::new();
    let size = u32::try_from(atom.len() + 8).unwrap();
    moov.extend_from_slice(&size.to_be_bytes());
    moov.extend_from_slice(b"moov");
    moov.extend_from_slice(&atom);

    let chapters = parse_mp4_chapters(&moov).expect("chapters");
    assert_eq!(chapters.len(), 2);
    assert_eq!(chapters[0].title, "Intro");
    assert_eq!(chapters[1].start_ms, 60_000);
}
