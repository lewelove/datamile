use std::path::PathBuf;

use rayon::prelude::*;

pub struct Library {
    pub root: PathBuf,
}

impl Library {
    pub const fn new(root: PathBuf) -> Self {
        Self { root }
    }

    pub fn scan(&self, logic_engine: &mut crate::server::logic::LogicEngine) {
        log::info!("Scanning Library at {}", self.root.display());

        let config_path = &logic_engine.config_path;

        let evaluated_items: Vec<_> = {
            let Ok(engine) = libdatamile::lua::LuaEngine::new() else {
                log::error!("Failed to create Lua engine for library scan");
                return;
            };
            if let Err(e) = engine.evaluate_config(config_path) {
                log::error!("Failed to evaluate config for library scan: {e}");
                return;
            }
            let Ok(roots) =
                crate::compile::media::discover_targets(&engine, &self.root, None)
            else {
                log::error!("Failed to discover roots under {}", self.root.display());
                return;
            };

            roots
                .into_par_iter()
                .map_init(
                    || match libdatamile::lua::LuaEngine::new() {
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
                    |engine_opt, root| {
                        let engine = engine_opt.as_ref()?;
                        let crate::compile::media::RootTarget {
                            root_type,
                            target,
                            lock_path,
                            ..
                        } = root;
                        let content = std::fs::read_to_string(&lock_path).ok()?;
                        let parsed: serde_json::Value =
                            serde_json::from_str(&content).ok()?;
                        let id = parsed
                            .get(&root_type)
                            .and_then(|entity| entity.get("id"))
                            .and_then(serde_json::Value::as_str)
                            .filter(|id| !id.is_empty())?
                            .to_string();
                        let eval_res =
                            engine.evaluate_entity_logic(&root_type, &parsed).ok()?;

                        Some((root_type, target, lock_path, id, content, eval_res))
                    },
                )
                .flatten()
                .collect()
        };

        logic_engine.clear();

        for (root_type, target, lock_path, id, content, eval_res) in evaluated_items {
            let ingest = crate::server::logic::EntityIngest {
                root_type: &root_type,
                target: &target,
                lock_path: &lock_path,
                id: &id,
                lock_json: &content,
                eval_res,
                media_directory: &self.root,
            };

            if let Err(e) = logic_engine.ingest_entity(ingest) {
                log::error!("Dedup validation error for {}: {e}", target.display());
            }
        }

        logic_engine.build_cache();
        log::info!("Library Logic Engine Initialized.");
    }
}
