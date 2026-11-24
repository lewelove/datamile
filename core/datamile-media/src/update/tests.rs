//! The typed event stream: rendering, serialization and the bounded per-client
//! queue.

use std::path::PathBuf;

use super::{PassResult, StreamSink, UpdateEvent, UpdateSink, parse_event};

fn added() -> UpdateEvent {
    UpdateEvent::Added {
        media_type: "album".to_string(),
        id: "Artist - A".to_string(),
        dir: PathBuf::from("/music/A"),
    }
}

fn finished(failed: usize) -> UpdateEvent {
    UpdateEvent::Finished {
        discovered: 2,
        added: 1,
        updated: 1,
        unchanged: 0,
        removed: 0,
        failed,
    }
}

/// Every event renders as one readable line.
#[test]
fn events_render_as_one_line_each() {
    assert_eq!(
        UpdateEvent::Started {
            discovered: 3,
            dirty: 1
        }
        .to_string(),
        "Verifying 3 roots (1 dirty)..."
    );
    assert_eq!(added().to_string(), "Added album 'Artist - A' (/music/A)");
    assert_eq!(
        UpdateEvent::Removed {
            media_type: "album".to_string(),
            dir: PathBuf::from("/music/A")
        }
        .to_string(),
        "Removed album (/music/A)"
    );
    assert_eq!(
        UpdateEvent::Failed {
            media_type: "album".to_string(),
            dir: None,
            error: "bad state".to_string()
        }
        .to_string(),
        "Failed album: bad state"
    );
    assert!(
        finished(1)
            .to_string()
            .starts_with("Finished: 2 discovered")
    );
}

/// A client stream carries one JSON event per line and ends when its queue is
/// full, so a slow client is unregistered instead of blocking the pass.
#[tokio::test]
async fn stream_sink_unregisters_a_full_queue() {
    let (tx, mut rx) = tokio::sync::mpsc::channel::<String>(1);
    let sink = StreamSink::new(tx);

    sink.event(&added());
    sink.event(&added());
    sink.event(&added());

    assert!(rx.recv().await.is_some(), "the first event fits");
    assert!(rx.recv().await.is_none(), "the stream ends after the drop");
}

/// A serialized `Finished` is what gives the CLI its exit status.
#[test]
fn finished_event_round_trips_through_the_stream() {
    let (tx, mut rx) = tokio::sync::mpsc::channel::<String>(4);
    let sink = StreamSink::new(tx);

    sink.event(&finished(2));

    let line = rx.try_recv().expect("line");
    let recorder = super::CompositeSink::new(Vec::new());
    let result = parse_event(&line, &recorder).expect("Finished");

    assert_eq!(
        result,
        PassResult {
            discovered: 2,
            added: 1,
            updated: 1,
            unchanged: 0,
            removed: 0,
            failed: 2,
        }
    );
    assert!(!result.ok(), "any failure makes the command non-zero");
}
