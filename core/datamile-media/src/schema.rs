use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use datamile_lua_domain::{DomainLuaEngine, LogicManifest};
use serde::Deserialize;

/// Static schema for one nested node definition.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct MediaNodeSchema {
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

/// Media type names become path segments under the cache, so the character set
/// is restricted to what is safe there.
fn valid_media_type_name(name: &str) -> bool {
    !name.is_empty()
        && name
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-')
}

/// Phase 0: validates the DSL before any `init` directory is touched.
pub fn validate_media_schema(
    manifest: &MediaManifest,
    logic: &LogicManifest,
) -> Result<()> {
    let mut seen = HashSet::new();
    for name in &manifest.roots_order {
        if !valid_media_type_name(name) {
            bail!(
                "media.root '{name}' must match [a-z0-9_-]+, so it is safe as a path segment"
            );
        }
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

/// Everything one discovery run produced. A media root that fails on its own is
/// listed in `failures` instead of aborting the whole pass.
#[derive(Debug, Default)]
pub struct Discovery {
    pub roots: Vec<DiscoveredRoot>,
    pub failures: Vec<(String, anyhow::Error)>,
}

/// Phase 1: discovers every root directory and enforces the per-media-type
/// no-nesting rule.
///
/// One directory may be claimed by any number of distinct media types; only
/// nesting inside one media type is forbidden.
pub fn discover_roots(
    engine: &DomainLuaEngine,
    manifest: &MediaManifest,
    config_dir: &Path,
) -> Result<Discovery> {
    let mut discovery = Discovery::default();

    for media_type in &manifest.roots_order {
        let Some(root) = manifest.roots.get(media_type) else {
            continue;
        };
        let Some(init) = root.init.as_deref() else {
            continue;
        };

        match select_media_roots(engine, media_type, init, config_dir) {
            Ok((init_path, dirs)) => {
                for dir in dirs {
                    discovery.roots.push(DiscoveredRoot {
                        media_type: media_type.clone(),
                        init: init_path.clone(),
                        dir,
                    });
                }
            }
            Err(error) => discovery.failures.push((media_type.clone(), error)),
        }
    }

    reject_nesting(&discovery.roots)?;

    Ok(discovery)
}

fn select_media_roots(
    engine: &DomainLuaEngine,
    media_type: &str,
    init: &str,
    config_dir: &Path,
) -> Result<(PathBuf, Vec<PathBuf>)> {
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
        .call((media_type, init_path.to_string_lossy()))
        .map_err(|e| anyhow::anyhow!("{e}"))?;
    let selected: Vec<String> = engine
        .runtime
        .from_value(selected_value)
        .context("select must return a list of paths")?;

    let mut seen: HashSet<PathBuf> = HashSet::new();
    let mut dirs = Vec::new();
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
        if seen.insert(dir.clone()) {
            dirs.push(dir);
        }
    }

    Ok((init_path, dirs))
}

/// Root entities of one media type may not nest, so every file belongs to at
/// most one root entity of that type.
///
/// Nesting across media types stays allowed: a film entity may contain its
/// soundtrack album entity.
fn reject_nesting(roots: &[DiscoveredRoot]) -> Result<()> {
    let mut sorted: Vec<&DiscoveredRoot> = roots.iter().collect();
    sorted.sort_by(|a, b| (&a.media_type, &a.dir).cmp(&(&b.media_type, &b.dir)));

    for pair in sorted.windows(2) {
        let [parent, child] = pair else {
            continue;
        };

        // Different media types may nest; only same-type nesting is an error.
        if parent.media_type != child.media_type {
            continue;
        }

        if child.dir.starts_with(&parent.dir) {
            bail!(
                "root entities of media type '{}' may not nest: {} is inside {}",
                parent.media_type,
                child.dir.display(),
                parent.dir.display()
            );
        }
    }

    Ok(())
}

/// Reads the data-only schema out of the Lua VM.
pub fn read_media_manifest(engine: &DomainLuaEngine) -> Result<MediaManifest> {
    engine
        .runtime
        .call_global("__DATAMILE_GET_MEDIA_MANIFEST")
        .context("Failed to read the media manifest")
}
