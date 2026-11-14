use super::*;
use crate::server::state::{DependencyGraph, RootMeta};

/// Untracked files mark their parent for shallow discovery; files inside a
/// watched directory stay on the existing lock path.
#[test]
fn untracked_marks_shallow() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let music = tmp.path().join("music");
    let untracked = music.join("untracked");
    let tracked = music.join("tracked");
    std::fs::create_dir_all(&untracked).expect("untracked");
    std::fs::create_dir_all(&tracked).expect("tracked");
    let marker = untracked.join("album.toml");
    std::fs::write(&marker, b"").expect("marker");

    let lock = tracked.join("album.lock.json");
    let mut graph = DependencyGraph::default();
    graph.update_lock(
        lock,
        RootMeta {
            root_type: "album".to_string(),
            target: tracked.clone(),
            files: HashSet::new(),
            dirs: HashSet::from([tracked.clone()]),
        },
    );

    let empty_deps = HashSet::new();
    let media_roots = [music];
    let ctx = ClassifierContext {
        config_dir: Path::new("/config"),
        media_roots: &media_roots,
        deps: &empty_deps,
        deps_graph: &graph,
    };
    let mut flags = ChangeFlags {
        config: false,
        changed_locks: HashSet::new(),
        changed_paths: HashSet::new(),
        shallow_paths: HashSet::new(),
        vanished_paths: HashSet::new(),
    };

    let canon = marker.canonicalize().expect("canonicalize");
    classify_path_change(&marker, &canon, &ctx, &mut flags);

    assert_eq!(
        flags.shallow_paths,
        HashSet::from([untracked.canonicalize().expect("canonicalize")])
    );
    assert!(flags.changed_locks.is_empty());
}
