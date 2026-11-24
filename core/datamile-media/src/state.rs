use std::collections::HashMap;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use datamile_cache::calculate_path_hash;
use datamile_lua_domain::{ActionDef, DomainLuaEngine, InterfaceDef, LogicManifest};
use datamile_mpd::MpdConfig;

use crate::schema::{DiscoveredRoot, MediaManifest, MediaRootSchema};

pub const MEDIA_LUA: &str = include_str!("media.lua");
pub const COMPILE_ROOT_LUA: &str = include_str!("compile_root.lua");

const DEFAULT_CACHE_ROOT: &str = "~/.cache/datamile";

/// Everything loaded from `init.lua` plus the discovered roots.
#[derive(Clone)]
pub struct MediaState {
    pub config_path: PathBuf,
    pub config_dir: PathBuf,
    pub cache_root: PathBuf,
    pub state_root: PathBuf,
    pub media: MediaManifest,
    pub logic_manifest: LogicManifest,
    pub roots: Vec<DiscoveredRoot>,
    pub dependencies: Vec<PathBuf>,
    pub actions: HashMap<String, ActionDef>,
    pub interfaces: HashMap<String, InterfaceDef>,
    pub mpd: MpdConfig,
    pub compiler_jobs: Option<usize>,
    /// `server.port`, normalised to the default when the config leaves it at 0.
    pub port: u16,
}

impl MediaState {
    #[must_use]
    pub fn root_schema(&self, media_type: &str) -> Option<&MediaRootSchema> {
        self.media.roots.get(media_type)
    }

    /// Root directories of one media type, used by the hot-reload guards.
    #[must_use]
    pub fn roots_of(&self, media_type: &str) -> Vec<PathBuf> {
        self.roots
            .iter()
            .filter(|root| root.media_type == media_type)
            .map(|root| root.dir.clone())
            .collect()
    }

    /// `(media type, canonical init)` for every declared media root, whether or
    /// not `select` currently returns any root entity. Removal needs the ones
    /// that return none.
    #[must_use]
    pub fn media_roots(&self) -> Vec<(String, PathBuf)> {
        self.media
            .roots_order
            .iter()
            .filter_map(|media_type| {
                let init = self.media.roots.get(media_type)?.init.as_deref()?;
                let init = datamile_fs::resolve_path(init, &self.config_dir);
                Some((media_type.clone(), canonical(&init)))
            })
            .collect()
    }

    /// Canonical `init` of one media root, needed to find its state file.
    #[must_use]
    pub fn init_of(&self, media_type: &str) -> Option<PathBuf> {
        self.media_roots()
            .into_iter()
            .find(|(name, _)| name == media_type)
            .map(|(_, init)| init)
    }

    #[must_use]
    pub fn init_dirs(&self) -> Vec<PathBuf> {
        let mut inits: Vec<PathBuf> =
            self.roots.iter().map(|root| root.init.clone()).collect();
        inits.sort();
        inits.dedup();
        inits
    }

    /// MPD base for a root type: `mpd.music_directory` or the root's own init.
    #[must_use]
    pub fn mpd_base(&self, media_type: &str, root_dir: &Path) -> Option<PathBuf> {
        if let Some(music_directory) = &self.mpd.music_directory {
            let expanded = datamile_fs::expand_path(music_directory);
            return Some(expanded.canonicalize().unwrap_or_else(|_| expanded.clone()));
        }
        self.roots
            .iter()
            .find(|root| root.media_type == media_type && root.dir == root_dir)
            .map(|root| root.init.clone())
    }
}

/// Opens a Lua VM with the media DSL installed and `init.lua` evaluated.
pub fn open_engine(state: &MediaState) -> Result<DomainLuaEngine> {
    let engine = DomainLuaEngine::new()?;
    engine.load_extension("media.lua", MEDIA_LUA)?;
    engine.load_extension("compile_root.lua", COMPILE_ROOT_LUA)?;
    crate::lua_bridge::register_native_helpers(&engine.runtime)?;
    engine
        .evaluate_config(&state.config_path)
        .context("Failed to evaluate init.lua for the media engine")?;
    Ok(engine)
}

/// Durable state is namespaced by config path, so two configs never share a
/// state file even when they share a cache root and an `init`.
#[must_use]
pub fn config_cache_dir(cache_root: &Path, config_path: &Path) -> PathBuf {
    cache_root
        .join("configs")
        .join(calculate_path_hash(&canonical(config_path)))
}

#[must_use]
pub fn config_hash_path(cache_root: &Path, config_path: &Path) -> PathBuf {
    config_cache_dir(cache_root, config_path).join("config.blake3")
}

/// The state file for one media root: one per `(config, media type, init)`.
#[must_use]
pub fn state_path(
    cache_root: &Path,
    config_path: &Path,
    media_type: &str,
    init: &Path,
) -> PathBuf {
    config_cache_dir(cache_root, config_path)
        .join("roots")
        .join(media_type)
        .join(calculate_path_hash(&canonical(init)))
        .join("state.json")
}

fn canonical(path: &Path) -> PathBuf {
    path.canonicalize().unwrap_or_else(|_| path.to_path_buf())
}

#[must_use]
pub fn default_cache_root() -> PathBuf {
    datamile_fs::expand_path(DEFAULT_CACHE_ROOT)
}
