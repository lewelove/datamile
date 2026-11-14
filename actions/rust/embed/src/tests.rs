use std::borrow::Cow;
use std::collections::HashMap;
use std::fs;

use lofty::id3::v2::{Frame, FrameId, Id3v2Tag};
use lofty::tag::{ItemKey, Tag, TagType};
use tempfile::tempdir;

use crate::models::{CoverDeleteMode, CoverStatus, TagDeleteMode, TrackTask};
use crate::tag::{extract_tag_map, read_file_tag, write_tasks};

const DUMMY_MP3: &[u8] = include_bytes!("../tests/fixtures/full_test.mp3");

#[test]
fn test_id3v2_ufid_roundtrip() {
    let rec_id = "01234567-89ab-cdef-0123-456789abcdef";
    let mut tag = Tag::new(TagType::Id3v2);
    tag.insert_text(ItemKey::MusicBrainzRecordingId, rec_id.to_string());

    let id3v2: Id3v2Tag = tag.into();
    let ufid_frame = id3v2.get(&FrameId::Valid(Cow::Borrowed("UFID")));
    assert!(matches!(ufid_frame, Some(Frame::UniqueFileIdentifier(_))));

    let roundtrip_tag: Tag = id3v2.into();
    let tag_map = extract_tag_map(&roundtrip_tag);
    assert_eq!(
        tag_map
            .get(&ItemKey::MusicBrainzRecordingId)
            .map(String::as_str),
        Some(rec_id)
    );
}

#[test]
fn test_id3v2_tipl_roundtrip() {
    let mut tag = Tag::new(TagType::Id3v2);
    tag.insert_text(ItemKey::Producer, "Test Producer".to_string());
    tag.insert_text(ItemKey::Arranger, "Test Arranger".to_string());
    tag.insert_text(ItemKey::Engineer, "Test Engineer".to_string());
    tag.insert_text(ItemKey::MixDj, "Test DJ".to_string());
    tag.insert_text(ItemKey::MixEngineer, "Test Mixer".to_string());

    let id3v2: Id3v2Tag = tag.into();
    let tipl_frame = id3v2.get(&FrameId::Valid(Cow::Borrowed("TIPL")));
    assert!(tipl_frame.is_some());

    let roundtrip_tag: Tag = id3v2.into();
    let tag_map = extract_tag_map(&roundtrip_tag);
    assert_eq!(
        tag_map.get(&ItemKey::Producer).map(String::as_str),
        Some("Test Producer")
    );
    assert_eq!(
        tag_map.get(&ItemKey::Arranger).map(String::as_str),
        Some("Test Arranger")
    );
    assert_eq!(
        tag_map.get(&ItemKey::Engineer).map(String::as_str),
        Some("Test Engineer")
    );
    assert_eq!(
        tag_map.get(&ItemKey::MixDj).map(String::as_str),
        Some("Test DJ")
    );
    assert_eq!(
        tag_map.get(&ItemKey::MixEngineer).map(String::as_str),
        Some("Test Mixer")
    );
}

#[test]
fn test_id3v2_mbids_and_color() {
    let mut tag = Tag::new(TagType::Id3v2);
    tag.insert_text(ItemKey::MusicBrainzTrackId, "mb-track-1".to_string());
    tag.insert_text(ItemKey::MusicBrainzReleaseId, "mb-release-2".to_string());
    tag.insert_text(ItemKey::MusicBrainzReleaseGroupId, "mb-rg-3".to_string());
    tag.insert_text(ItemKey::MusicBrainzArtistId, "mb-artist-4".to_string());
    tag.insert_text(ItemKey::Color, "#334455".to_string());

    let id3v2: Id3v2Tag = tag.into();
    let roundtrip_tag: Tag = id3v2.into();
    let tag_map = extract_tag_map(&roundtrip_tag);

    assert_eq!(
        tag_map
            .get(&ItemKey::MusicBrainzTrackId)
            .map(String::as_str),
        Some("mb-track-1")
    );
    assert_eq!(
        tag_map
            .get(&ItemKey::MusicBrainzReleaseId)
            .map(String::as_str),
        Some("mb-release-2")
    );
    assert_eq!(
        tag_map
            .get(&ItemKey::MusicBrainzReleaseGroupId)
            .map(String::as_str),
        Some("mb-rg-3")
    );
    assert_eq!(
        tag_map
            .get(&ItemKey::MusicBrainzArtistId)
            .map(String::as_str),
        Some("mb-artist-4")
    );
    assert_eq!(
        tag_map.get(&ItemKey::Color).map(String::as_str),
        Some("#334455")
    );
}

#[test]
fn test_id3v2_lyrics_roundtrip() {
    let lyrics_content = "Verse 1\nLine 2";
    let mut tag = Tag::new(TagType::Id3v2);
    tag.insert_text(ItemKey::UnsyncLyrics, lyrics_content.to_string());

    let id3v2: Id3v2Tag = tag.into();
    let uslt_frame = id3v2.get(&FrameId::Valid(Cow::Borrowed("USLT")));
    assert!(matches!(uslt_frame, Some(Frame::UnsynchronizedText(_))));

    let roundtrip_tag: Tag = id3v2.into();
    let tag_map = extract_tag_map(&roundtrip_tag);
    assert_eq!(
        tag_map.get(&ItemKey::Lyrics).map(String::as_str),
        Some(lyrics_content)
    );
}

#[test]
fn test_file_tasks_roundtrip() {
    let dir = tempdir().expect("Failed to create temp directory");
    let file_path = dir.path().join("track.mp3");
    fs::write(&file_path, DUMMY_MP3).expect("Failed to write dummy MP3 file");

    let mut target_tags = HashMap::new();
    target_tags.insert(ItemKey::AlbumTitle, "Test Album".to_string());
    target_tags.insert(ItemKey::TrackTitle, "Test Title".to_string());
    target_tags.insert(ItemKey::TrackNumber, "2".to_string());
    target_tags.insert(ItemKey::DiscNumber, "1".to_string());
    target_tags.insert(ItemKey::Producer, "Lead Producer".to_string());
    target_tags.insert(ItemKey::Lyrics, "Song lyrics text".to_string());
    target_tags.insert(ItemKey::MusicBrainzRecordingId, "mb-rec-99".to_string());
    target_tags.insert(ItemKey::MusicBrainzTrackId, "mb-trk-88".to_string());
    target_tags.insert(ItemKey::MusicBrainzReleaseId, "mb-rel-77".to_string());
    target_tags.insert(ItemKey::MusicBrainzReleaseGroupId, "mb-rg-66".to_string());
    target_tags.insert(ItemKey::Color, "#AABBCC".to_string());

    let task = TrackTask {
        path: file_path.clone(),
        target_tags,
        diffs: Vec::new(),
    };

    write_tasks(
        &[task],
        CoverStatus::Preserve,
        None,
        TagDeleteMode::DeleteOther,
        CoverDeleteMode::PreserveOther,
    )
    .expect("write_tasks failed");

    let (read_tag, _) = read_file_tag(&file_path).expect("read_file_tag failed");
    let tag_map = extract_tag_map(&read_tag);

    assert_eq!(
        tag_map.get(&ItemKey::AlbumTitle).map(String::as_str),
        Some("Test Album")
    );
    assert_eq!(
        tag_map.get(&ItemKey::TrackTitle).map(String::as_str),
        Some("Test Title")
    );
    assert_eq!(
        tag_map.get(&ItemKey::TrackNumber).map(String::as_str),
        Some("2")
    );
    assert_eq!(
        tag_map.get(&ItemKey::DiscNumber).map(String::as_str),
        Some("1")
    );
    assert_eq!(
        tag_map.get(&ItemKey::Producer).map(String::as_str),
        Some("Lead Producer")
    );
    assert_eq!(
        tag_map.get(&ItemKey::Lyrics).map(String::as_str),
        Some("Song lyrics text")
    );
    assert_eq!(
        tag_map
            .get(&ItemKey::MusicBrainzRecordingId)
            .map(String::as_str),
        Some("mb-rec-99")
    );
    assert_eq!(
        tag_map
            .get(&ItemKey::MusicBrainzTrackId)
            .map(String::as_str),
        Some("mb-trk-88")
    );
    assert_eq!(
        tag_map
            .get(&ItemKey::MusicBrainzReleaseId)
            .map(String::as_str),
        Some("mb-rel-77")
    );
    assert_eq!(
        tag_map
            .get(&ItemKey::MusicBrainzReleaseGroupId)
            .map(String::as_str),
        Some("mb-rg-66")
    );
    assert_eq!(
        tag_map.get(&ItemKey::Color).map(String::as_str),
        Some("#AABBCC")
    );
}

#[test]
fn test_file_delete_other_tags() {
    let dir = tempdir().expect("Failed to create temp directory");
    let file_path = dir.path().join("track.mp3");
    fs::write(&file_path, DUMMY_MP3).expect("Failed to write dummy MP3 file");

    let mut initial_tags = HashMap::new();
    initial_tags.insert(ItemKey::AlbumTitle, "Initial Album".to_string());
    initial_tags.insert(ItemKey::Comment, "Deprecated Comment".to_string());
    initial_tags.insert(ItemKey::EncodedBy, "Preserved Encoder".to_string());

    let initial_task = TrackTask {
        path: file_path.clone(),
        target_tags: initial_tags,
        diffs: Vec::new(),
    };

    write_tasks(
        &[initial_task],
        CoverStatus::Preserve,
        None,
        TagDeleteMode::DeleteOther,
        CoverDeleteMode::PreserveOther,
    )
    .expect("initial write_tasks failed");

    let mut updated_tags = HashMap::new();
    updated_tags.insert(ItemKey::AlbumTitle, "Updated Album".to_string());

    let updated_task = TrackTask {
        path: file_path.clone(),
        target_tags: updated_tags,
        diffs: Vec::new(),
    };

    write_tasks(
        &[updated_task],
        CoverStatus::Preserve,
        None,
        TagDeleteMode::DeleteOther,
        CoverDeleteMode::PreserveOther,
    )
    .expect("updated write_tasks failed");

    let (read_tag, _) = read_file_tag(&file_path).expect("read_file_tag failed");
    let tag_map = extract_tag_map(&read_tag);

    assert_eq!(
        tag_map.get(&ItemKey::AlbumTitle).map(String::as_str),
        Some("Updated Album")
    );
    assert_eq!(
        tag_map.get(&ItemKey::EncodedBy).map(String::as_str),
        Some("Preserved Encoder")
    );
    assert!(!tag_map.contains_key(&ItemKey::Comment));
}
