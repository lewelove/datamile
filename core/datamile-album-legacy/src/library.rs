use std::path::PathBuf;

use rayon::prelude::*;
use walkdir::WalkDir;

pub struct Library {
    pub root: PathBuf,
}

impl Library {
    pub const fn new(root: PathBuf) -> Self {
        Self { root }
    }

    pub fn scan(&self, logic_engine: &mut datamile_logic::LogicEngine) {
        log::info!("Scanning Library at {}", self.root.display());

        let lock_paths: Vec<PathBuf> = WalkDir::new(&self.root)
            .follow_links(true)
            .into_iter()
            .filter_map(Result::ok)
            .filter(|e| e.file_name() == "album.lock.json")
            .map(|e| e.path().to_path_buf())
            .collect();

        let config_path = &logic_engine.config_path;

        let evaluated_items: Vec<_> = lock_paths
            .into_par_iter()
            .map_init(
                || match crate::lua::LuaEngine::new() {
                    Ok(engine) => match engine.evaluate_config(config_path) {
                        Ok(_) => Some(engine),
                        Err(e) => {
                            log::error!(
                                "Failed to evaluate config for scanner thread: {e}"
                            );
                            None
                        }
                    },
                    Err(e) => {
                        log::error!(
                            "Failed to initialize Lua engine for scanner thread: {e}"
                        );
                        None
                    }
                },
                |engine_opt, lock_path| {
                    let engine = engine_opt.as_ref()?;
                    let canon = lock_path
                        .canonicalize()
                        .unwrap_or_else(|_| lock_path.clone());
                    let canon_display = canon.display();
                    let content = match std::fs::read_to_string(&lock_path) {
                        Ok(c) => c,
                        Err(e) => {
                            log::error!("Failed to read {canon_display}: {e}");
                            return None;
                        }
                    };
                    let album_dir =
                        lock_path.parent().unwrap_or(&lock_path).to_path_buf();
                    let album_dir_canon = album_dir
                        .canonicalize()
                        .unwrap_or_else(|_| album_dir.clone());
                    let album_dir_display = album_dir_canon.display();

                    let parsed: serde_json::Value = match serde_json::from_str(&content) {
                        Ok(p) => p,
                        Err(e) => {
                            log::error!(
                                "Failed to parse JSON content for {canon_display}: {e}"
                            );
                            return None;
                        }
                    };

                    let alb_id = match parsed
                        .pointer("/album/id")
                        .and_then(serde_json::Value::as_str)
                    {
                        Some(id) if !id.is_empty() => id.to_string(),
                        _ => {
                            log::error!("Missing or invalid album id in {canon_display}");
                            return None;
                        }
                    };

                    let eval_res = match engine.evaluate_album_logic(&parsed) {
                        Ok(eval) => eval,
                        Err(e) => {
                            log::error!(
                                "Logic evaluation failed for {album_dir_display}: {e}"
                            );
                            return None;
                        }
                    };

                    Some((album_dir, alb_id, content, eval_res))
                },
            )
            .flatten()
            .collect();

        logic_engine.clear();
        ingest_evaluated(&self.root, evaluated_items, logic_engine);

        crate::lua_bridge::rebuild_cache(logic_engine);
        log::info!("Library Logic Engine Initialized.");
    }
}

/// Registers every evaluated lock, logging and skipping the ones that fail.
fn ingest_evaluated(
    root: &std::path::Path,
    items: Vec<(PathBuf, String, String, serde_json::Value)>,
    logic_engine: &mut datamile_logic::LogicEngine,
) {
    for (album_dir, alb_id, content, eval_res) in items {
        let album_dir_canon = album_dir
            .canonicalize()
            .unwrap_or_else(|_| album_dir.clone());
        let parsed =
            match crate::ingest::parse_album_lock(&album_dir, &alb_id, &content, root) {
                Ok(Some(parsed)) => parsed,
                Ok(None) => continue,
                Err(e) => {
                    let album_dir_display = album_dir_canon.display();
                    log::error!("Failed to parse lock for {album_dir_display}: {e}");
                    continue;
                }
            };

        if let Err(e) = logic_engine.upsert_entity(
            parsed.entity,
            parsed.playables,
            parsed.images,
            eval_res,
        ) {
            let album_dir_display = album_dir_canon.display();
            log::error!("Dedup validation error for {album_dir_display}: {e}");
        }
    }
}
