use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use datamile_lua_domain::{DomainLuaEngine, LogicManifest};
use serde::Deserialize;

/// Static schema for one nested node definition.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct MediaNodeSchema {
    #[serde(default)]
    pub playable: bool,
    #[serde(default)]
    pub has_select: bool,
    #[serde(default)]
    pub nodes: BTreeMap<String, Self>,
}

/// Static schema for one `d.media.root`.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct MediaRootSchema {
    #[serde(default)]
    pub init: Option<String>,
    #[serde(default)]
    pub has_select: bool,
    #[serde(default)]
    pub has_id: bool,
    #[serde(default)]
    pub nodes: BTreeMap<String, MediaNodeSchema>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct MediaManifest {
    #[serde(default)]
    pub roots_order: Vec<String>,
    #[serde(default)]
    pub roots: BTreeMap<String, MediaRootSchema>,
}

#[derive(Debug, Clone)]
pub struct DiscoveredRoot {
    pub media_type: String,
    pub init: PathBuf,
    pub dir: PathBuf,
}

fn validate_nodes(
    media_type: &str,
    prefix: &str,
    nodes: &BTreeMap<String, MediaNodeSchema>,
) -> Result<()> {
    for (name, node) in nodes {
        if !node.has_select {
            bail!(
                "media.root '{media_type}' node '{prefix}{name}' is missing a select function"
            );
        }
        validate_nodes(media_type, &format!("{prefix}{name}."), &node.nodes)?;
    }
    Ok(())
}

/// Phase 0: validates the DSL before any `init` directory is touched.
pub fn validate_media_schema(
    manifest: &MediaManifest,
    logic: &LogicManifest,
) -> Result<()> {
    let mut seen = HashSet::new();
    for name in &manifest.roots_order {
        if !seen.insert(name.clone()) {
            bail!("duplicate media.root name '{name}'");
        }

        let Some(root) = manifest.roots.get(name) else {
            bail!("media.root '{name}' is missing from the registry");
        };

        if root.init.as_deref().is_none_or(str::is_empty) {
            bail!("media.root '{name}' has an empty or missing 'init' path");
        }
        if !root.has_select {
            bail!("media.root '{name}' is missing a select function");
        }
        if !root.has_id {
            bail!("media.root '{name}' is missing an id function");
        }
        validate_nodes(name, "", &root.nodes)?;
    }

    for (name, def) in &logic.libraries {
        require_media(name, "library", &def.media, manifest)?;
    }
    for (name, def) in &logic.filters {
        require_media(name, "filter", &def.media, manifest)?;
    }
    for (name, def) in &logic.groupers {
        require_media(name, "grouper", &def.media, manifest)?;
    }
    for (name, def) in &logic.orders {
        require_media(name, "order", &def.media, manifest)?;
    }
    for (name, def) in &logic.shelves {
        require_media(name, "shelf", &def.media, manifest)?;
    }
    for (name, def) in &logic.cabinets {
        require_media(name, "cabinet", &def.media, manifest)?;
    }

    Ok(())
}

fn require_media(
    name: &str,
    kind: &str,
    media: &str,
    manifest: &MediaManifest,
) -> Result<()> {
    if !manifest.roots.contains_key(media) {
        bail!("{kind} '{name}' declares unknown media type '{media}'");
    }
    Ok(())
}

/// Phase 1: discovers every root directory and enforces global uniqueness.
pub fn discover_roots(
    engine: &DomainLuaEngine,
    manifest: &MediaManifest,
    config_dir: &Path,
) -> Result<Vec<DiscoveredRoot>> {
    let mut discovered = Vec::new();
    let mut claimed: HashMap<PathBuf, String> = HashMap::new();

    for media_type in &manifest.roots_order {
        let Some(root) = manifest.roots.get(media_type) else {
            continue;
        };
        let Some(init) = root.init.as_deref() else {
            continue;
        };

        let init_path = datamile_fs::resolve_path(init, config_dir)
            .canonicalize()
            .with_context(|| {
                format!("media.root '{media_type}' init is not a directory: {init}")
            })?;

        let select_fn: mlua::Function = engine
            .runtime
            .lua
            .globals()
            .get("__DATAMILE_MEDIA_SELECT")
            .map_err(|e| anyhow::anyhow!("{e}"))?;
        let selected_value: mlua::Value = select_fn
            .call((media_type.as_str(), init_path.to_string_lossy()))
            .map_err(|e| anyhow::anyhow!("{e}"))?;
        let selected: Vec<String> = engine
            .runtime
            .from_value(selected_value)
            .context("select must return a list of paths")?;

        let mut seen: HashSet<PathBuf> = HashSet::new();
        for raw in selected {
            let dir = PathBuf::from(&raw)
                .canonicalize()
                .with_context(|| format!("selected root vanished: {raw}"))?;
            if !dir.is_dir() {
                bail!(
                    "media.root '{media_type}' selected a non-directory: {}",
                    dir.display()
                );
            }
            if !seen.insert(dir.clone()) {
                continue;
            }

            if let Some(owner) = claimed.get(&dir) {
                bail!(
                    "root directory {} is claimed by both '{owner}' and '{media_type}'",
                    dir.display()
                );
            }
            claimed.insert(dir.clone(), media_type.clone());

            discovered.push(DiscoveredRoot {
                media_type: media_type.clone(),
                init: init_path.clone(),
                dir,
            });
        }
    }

    Ok(discovered)
}

/// Reads the data-only schema out of the Lua VM.
pub fn read_media_manifest(engine: &DomainLuaEngine) -> Result<MediaManifest> {
    engine
        .runtime
        .call_global("__DATAMILE_GET_MEDIA_MANIFEST")
        .context("Failed to read the media manifest")
}
