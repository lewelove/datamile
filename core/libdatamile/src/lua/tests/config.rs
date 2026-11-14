use std::path::PathBuf;

use crate::lua::LuaEngine;

/// Verify both starter configs load without relying on binary-baked defaults.
#[test]
fn test_starter_examples_load() {
    for name in ["music-basic", "music-manifestless"] {
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../examples")
            .join(name)
            .join("init.lua");

        let engine = LuaEngine::new().expect("engine");
        engine.evaluate_config(&path).expect("evaluate starter");

        let roots = engine.media_roots().expect("roots");
        assert_eq!(roots, vec!["album".to_string()], "example {name}");
    }
}
