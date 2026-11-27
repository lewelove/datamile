use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use anyhow::{Context, Result, bail};
use datamile_cache::{EntityKey, load_state, root_is_dirty, walk_files};
use datamile_logic::LogicEngine;
use datamile_server::{ServerState, UpdateRequest};
use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::compiler::{CompiledRoot, compile_root};
use crate::deps::{
    MediaShared, UpdateScope, index_entity, state_entries, unindex_entity,
};
use crate::schema::{DiscoveredRoot, Discovery, discover_roots};
use crate::state::{MediaState, open_engine, state_path};

#[cfg(test)]
mod tests;

/// One typed line of a pass. Rendered through `Display` and delivered to the
/// daemon log, the CLI and the HTTP response.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum UpdateEvent {
    Started {
        discovered: usize,
        dirty: usize,
    },
    Added {
        media_type: String,
        id: String,
        dir: PathBuf,
    },
    Updated {
        media_type: String,
        id: String,
        dir: PathBuf,
    },
    Unchanged {
        media_type: String,
        id: String,
        dir: PathBuf,
    },
    Removed {
        media_type: String,
        dir: PathBuf,
    },
    Failed {
        media_type: String,
        dir: Option<PathBuf>,
        error: String,
    },
    Finished {
        discovered: usize,
        added: usize,
        updated: usize,
        unchanged: usize,
        removed: usize,
        failed: usize,
    },
}

impl std::fmt::Display for UpdateEvent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Started { discovered, dirty } => {
                write!(f, "Verifying {discovered} roots ({dirty} dirty)...")
            }
            Self::Added {
                media_type,
                id,
                dir,
            } => write!(f, "Added {media_type} '{id}' ({})", dir.display()),
            Self::Updated {
                media_type,
                id,
                dir,
            } => write!(f, "Updated {media_type} '{id}' ({})", dir.display()),
            Self::Unchanged {
                media_type,
                id,
                dir,
            } => write!(f, "Unchanged {media_type} '{id}' ({})", dir.display()),
            Self::Removed { media_type, dir } => {
                write!(f, "Removed {media_type} ({})", dir.display())
            }
            Self::Failed {
                media_type,
                dir: Some(dir),
                error,
            } => write!(f, "Failed {media_type} ({}): {error}", dir.display()),
            Self::Failed {
                media_type,
                dir: None,
                error,
            } => write!(f, "Failed {media_type}: {error}"),
            Self::Finished {
                discovered,
                added,
                updated,
                unchanged,
                removed,
                failed,
            } => write!(
                f,
                "Finished: {discovered} discovered, {added} added, {updated} updated, \
                 {unchanged} unchanged, {removed} removed, {failed} failed."
            ),
        }
    }
}

/// Anything that receives every event of a pass.
pub trait UpdateSink: Send + Sync {
    fn event(&self, event: &UpdateEvent);
}

/// Writes every event to the logger at its own level.
pub struct LogSink;

impl UpdateSink for LogSink {
    fn event(&self, event: &UpdateEvent) {
        match event {
            UpdateEvent::Failed { .. } => log::error!("{event}"),
            UpdateEvent::Unchanged { .. } => log::debug!("{event}"),
            _ => log::info!("{event}"),
        }
    }
}

/// Forwards serialized events to one HTTP client. The queue is bounded: a
/// client that cannot keep up is unregistered and its stream ends.
pub struct StreamSink {
    tx: Mutex<Option<tokio::sync::mpsc::Sender<String>>>,
}

impl StreamSink {
    #[must_use]
    pub const fn new(tx: tokio::sync::mpsc::Sender<String>) -> Self {
        Self {
            tx: Mutex::new(Some(tx)),
        }
    }
}

impl UpdateSink for StreamSink {
    fn event(&self, event: &UpdateEvent) {
        let Ok(mut guard) = self.tx.lock() else {
            return;
        };
        let Some(tx) = guard.as_ref() else {
            return;
        };

        let Ok(line) = serde_json::to_string(event) else {
            return;
        };

        if tx.try_send(line).is_err() {
            // Full or closed: unregister so the stream ends instead of lying.
            *guard = None;
        }
    }
}

/// A sink made of sinks: one event reaches the journal and every attached
/// client without the pass knowing how many consumers there are.
pub struct CompositeSink {
    sinks: Vec<Arc<dyn UpdateSink>>,
}

impl CompositeSink {
    #[must_use]
    pub const fn new(sinks: Vec<Arc<dyn UpdateSink>>) -> Self {
        Self { sinks }
    }
}

impl UpdateSink for CompositeSink {
    fn event(&self, event: &UpdateEvent) {
        for sink in &self.sinks {
            sink.event(event);
        }
    }
}

/// Whether a pass must recompile everything or only dirty roots.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ForceMode {
    Force,
    Preserve,
}

impl From<bool> for ForceMode {
    fn from(force: bool) -> Self {
        if force { Self::Force } else { Self::Preserve }
    }
}

/// Counts per outcome. The command's exit status comes from `failed`, never from
/// counting received lines.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PassResult {
    pub discovered: usize,
    pub added: usize,
    pub updated: usize,
    pub unchanged: usize,
    pub removed: usize,
    pub failed: usize,
}

impl PassResult {
    #[must_use]
    pub const fn ok(&self) -> bool {
        self.failed == 0
    }
}

pub struct PassOutcome {
    pub result: PassResult,
    pub compiled: Vec<CompiledRoot>,
    /// Entities whose lock bytes changed or that are new. Only these are
    /// broadcast, so a re-verified root stays quiet for clients.
    pub changed: BTreeSet<EntityKey>,
    pub removed: Vec<(String, PathBuf)>,
}

/// One media root's state, loaded before anything is compiled.
struct MediaRootPlan {
    media_type: String,
    states: datamile_cache::RootStates,
}

type PerTypeUpdates = HashMap<
    String,
    (
        HashMap<String, Box<serde_json::value::RawValue>>,
        Vec<String>,
    ),
>;

fn in_scope(root: &DiscoveredRoot, scope: &UpdateScope) -> bool {
    let covers_type = |media_type: &Option<String>| {
        media_type
            .as_ref()
            .is_none_or(|name| name == &root.media_type)
    };

    match scope {
        UpdateScope::All { media_type } => covers_type(media_type),
        UpdateScope::Paths { paths, media_type } => {
            covers_type(media_type)
                && paths
                    .iter()
                    .any(|path| root.dir.starts_with(path) || path.starts_with(&root.dir))
        }
        UpdateScope::Entities(entities) => entities
            .iter()
            .any(|key| key.media_type == root.media_type && key.dir == root.dir),
    }
}

fn outcome_event(
    compiled: &CompiledRoot,
    known: &HashSet<(String, PathBuf)>,
) -> UpdateEvent {
    let media_type = compiled.media_type.clone();
    let id = compiled.id.clone();
    let dir = compiled.dir.clone();

    if !known.contains(&(media_type.clone(), dir.clone())) {
        return UpdateEvent::Added {
            media_type,
            id,
            dir,
        };
    }

    if compiled.modified {
        UpdateEvent::Updated {
            media_type,
            id,
            dir,
        }
    } else {
        UpdateEvent::Unchanged {
            media_type,
            id,
            dir,
        }
    }
}

fn compile_roots(
    state: &MediaState,
    dirty: &[DiscoveredRoot],
    jobs: Option<usize>,
    active_writes: &Arc<Mutex<HashSet<PathBuf>>>,
    known: &HashSet<(String, PathBuf)>,
    sink: &dyn UpdateSink,
) -> (Vec<CompiledRoot>, usize) {
    let default_jobs =
        std::thread::available_parallelism().map_or(1, std::num::NonZero::get);
    let Ok(pool) = rayon::ThreadPoolBuilder::new()
        .num_threads(jobs.unwrap_or(default_jobs).max(1))
        .build()
    else {
        let failed = dirty.len();
        for root in dirty {
            sink.event(&UpdateEvent::Failed {
                media_type: root.media_type.clone(),
                dir: Some(root.dir.clone()),
                error: "failed to build the compile pool".to_string(),
            });
        }
        return (Vec::new(), failed);
    };

    let failures = Mutex::new(0usize);

    let compiled = pool.install(|| {
        dirty
            .par_iter()
            .filter_map(|root| {
                let failed = |error: String| {
                    sink.event(&UpdateEvent::Failed {
                        media_type: root.media_type.clone(),
                        dir: Some(root.dir.clone()),
                        error,
                    });
                    if let Ok(mut count) = failures.lock() {
                        *count += 1;
                    }
                    None
                };

                let engine = match open_engine(state) {
                    Ok(engine) => engine,
                    Err(e) => return failed(format!("failed to open a compile VM: {e}")),
                };

                match compile_root(
                    &engine,
                    &root.media_type,
                    &root.dir,
                    &state.cache_root,
                    Some(active_writes),
                ) {
                    Ok(compiled) => {
                        sink.event(&outcome_event(&compiled, known));
                        Some(compiled)
                    }
                    Err(e) => failed(e.to_string()),
                }
            })
            .collect()
    });

    let failures = failures
        .into_inner()
        .unwrap_or_else(std::sync::PoisonError::into_inner);

    (compiled, failures)
}

/// Loads every media root's state file.
///
/// A media root that fails on its own is reported without a dir and skipped; the
/// rest of the pass continues.
fn load_plans(
    state: &MediaState,
    discovery: &Discovery,
    sink: &dyn UpdateSink,
    result: &mut PassResult,
) -> Vec<MediaRootPlan> {
    let mut broken: HashSet<String> = HashSet::new();

    for (media_type, error) in &discovery.failures {
        sink.event(&UpdateEvent::Failed {
            media_type: media_type.clone(),
            dir: None,
            error: error.to_string(),
        });
        result.failed += 1;
        broken.insert(media_type.clone());
    }

    let mut plans = Vec::new();
    for (media_type, init) in state.media_roots() {
        if broken.contains(&media_type) {
            continue;
        }

        let path = state_path(&state.cache_root, &state.config_path, &media_type, &init);

        match load_state(&path) {
            Ok(states) => plans.push(MediaRootPlan { media_type, states }),
            Err(error) => {
                sink.event(&UpdateEvent::Failed {
                    media_type: media_type.clone(),
                    dir: None,
                    error: error.to_string(),
                });
                result.failed += 1;
                broken.insert(media_type);
            }
        }
    }

    plans
}

/// What the pass will compile and what it must drop.
struct Planned {
    dirty: Vec<DiscoveredRoot>,
    removals: Vec<(String, PathBuf)>,
    /// Root entities that had a state entry before this pass.
    known: HashSet<(String, PathBuf)>,
    /// Root entities in scope, which is what `Started` reports.
    scoped: usize,
}

fn plan_work(
    plans: &[MediaRootPlan],
    roots: &[DiscoveredRoot],
    scope: &UpdateScope,
    force: ForceMode,
) -> Planned {
    let mut planned = Planned {
        dirty: Vec::new(),
        removals: Vec::new(),
        known: HashSet::new(),
        scoped: 0,
    };

    for plan in plans {
        let found: Vec<&DiscoveredRoot> = roots
            .iter()
            .filter(|root| root.media_type == plan.media_type)
            .collect();

        for dir in plan.states.keys() {
            // The key set is the list of root entities that existed at the end
            // of the last pass.
            planned
                .known
                .insert((plan.media_type.clone(), PathBuf::from(dir)));

            if !found.iter().any(|root| root.dir == Path::new(dir)) {
                planned
                    .removals
                    .push((plan.media_type.clone(), PathBuf::from(dir)));
            }
        }

        for root in found {
            if !in_scope(root, scope) {
                continue;
            }
            planned.scoped += 1;

            let entry = plan.states.get(&root.dir.to_string_lossy().to_string());
            let walked = walk_files(&root.dir);
            if force == ForceMode::Force
                || root_is_dirty(&root.dir, &root.media_type, &walked, entry)
            {
                planned.dirty.push(root.clone());
            }
        }
    }

    planned
}

/// Writes the state entries and the reverse index the watcher reads.
///
/// One write per media type, not one per root entity: a full pass over N roots
/// would otherwise load and save an N-entry file N times.
fn record_state(
    shared: &Arc<MediaShared>,
    state: &MediaState,
    compiled: &[CompiledRoot],
    removals: &[(String, PathBuf)],
    sink: &dyn UpdateSink,
    result: &mut PassResult,
) {
    let mut index = shared
        .external_index
        .write()
        .unwrap_or_else(std::sync::PoisonError::into_inner);

    for root in compiled {
        index_entity(
            &mut index,
            &EntityKey {
                media_type: root.media_type.clone(),
                dir: root.dir.clone(),
            },
            &root.dependencies,
        );
    }

    for (media_type, entries) in state_entries(compiled) {
        let dirs: Vec<PathBuf> = entries.iter().map(|(dir, ..)| dir.clone()).collect();

        if let Err(e) = shared.record_roots(state, &media_type, entries) {
            for dir in dirs {
                sink.event(&UpdateEvent::Failed {
                    media_type: media_type.clone(),
                    dir: Some(dir),
                    error: format!("failed to record the state file: {e}"),
                });
                result.failed += 1;
            }
        }
    }

    let mut by_type: BTreeMap<&str, Vec<&Path>> = BTreeMap::new();
    for (media_type, dir) in removals {
        unindex_entity(
            &mut index,
            &EntityKey {
                media_type: media_type.clone(),
                dir: dir.clone(),
            },
        );
        by_type
            .entry(media_type.as_str())
            .or_default()
            .push(dir.as_path());
    }

    for (media_type, dirs) in by_type {
        // A removal is reported only once the entry is really gone, or the next
        // pass would report the same root as removed again.
        match shared.remove_roots(state, media_type, &dirs) {
            Ok(()) => {
                for dir in dirs {
                    sink.event(&UpdateEvent::Removed {
                        media_type: media_type.to_string(),
                        dir: dir.to_path_buf(),
                    });
                    result.removed += 1;
                }
            }
            Err(e) => {
                for dir in dirs {
                    sink.event(&UpdateEvent::Failed {
                        media_type: media_type.to_string(),
                        dir: Some(dir.to_path_buf()),
                        error: format!("failed to record the state file: {e}"),
                    });
                    result.failed += 1;
                }
            }
        }
    }
}

/// One discovery, compile and state update pass.
///
/// Discovery is always full: `--path` and `--type` narrow compilation only, so
/// removals are reported whatever the scope.
pub fn run_pass(
    shared: &Arc<MediaShared>,
    scope: &UpdateScope,
    force: ForceMode,
    jobs: Option<usize>,
    sink: &dyn UpdateSink,
) -> Result<PassOutcome> {
    let state = shared
        .state
        .read()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .clone()
        .context("Media state is not loaded")?;

    let engine = open_engine(&state)?;
    let discovery = discover_roots(&engine, &state.media, &state.config_dir)?;

    let mut result = PassResult::default();
    let plans = load_plans(&state, &discovery, sink, &mut result);

    let Planned {
        dirty,
        removals,
        known,
        scoped,
    } = plan_work(&plans, &discovery.roots, scope, force);

    // A `--path` that matches no root entity would otherwise report a clean
    // pass and exit zero having done nothing. An entity key that vanished is a
    // removal, not a bad request.
    if matches!(scope, UpdateScope::Paths { .. }) && scoped == 0 && removals.is_empty() {
        bail!("no root entity matches the given path");
    }

    result.discovered = scoped;
    sink.event(&UpdateEvent::Started {
        discovered: scoped,
        dirty: dirty.len(),
    });

    let active_writes = Arc::new(Mutex::new(HashSet::new()));
    let jobs = jobs.or(state.compiler_jobs);
    let (compiled, failures) =
        compile_roots(&state, &dirty, jobs, &active_writes, &known, sink);
    result.failed += failures;

    let changed: BTreeSet<EntityKey> = compiled
        .iter()
        .filter(|root| {
            root.modified || !known.contains(&(root.media_type.clone(), root.dir.clone()))
        })
        .map(|root| EntityKey {
            media_type: root.media_type.clone(),
            dir: root.dir.clone(),
        })
        .collect();

    {
        let mut writes = shared
            .active_writes
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        writes.extend(
            active_writes
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .iter()
                .cloned(),
        );
    }

    record_state(shared, &state, &compiled, &removals, sink, &mut result);

    for compiled in &compiled {
        if known.contains(&(compiled.media_type.clone(), compiled.dir.clone())) {
            if compiled.modified {
                result.updated += 1;
            } else {
                result.unchanged += 1;
            }
        } else {
            result.added += 1;
        }
    }

    // Roots that were verified and found clean are `unchanged` too, so the
    // counts add up to `discovered` instead of all reading zero on a clean pass.
    result.unchanged += scoped.saturating_sub(dirty.len());

    sink.event(&UpdateEvent::Finished {
        discovered: result.discovered,
        added: result.added,
        updated: result.updated,
        unchanged: result.unchanged,
        removed: result.removed,
        failed: result.failed,
    });

    Ok(PassOutcome {
        result,
        compiled,
        changed,
        removed: removals,
    })
}

fn ingest_outcome(
    state: &MediaState,
    logic: &mut LogicEngine,
    outcome: &PassOutcome,
) -> PerTypeUpdates {
    let mut per_type = PerTypeUpdates::new();

    for (media_type, dir) in &outcome.removed {
        if let Some(id) = logic.remove_entity_by_type_and_path(media_type, dir) {
            per_type.entry(media_type.clone()).or_default().1.push(id);
        }
    }

    let Ok(engine) = open_engine(state) else {
        log::error!("Failed to open the media engine for ingestion");
        return per_type;
    };

    for compiled in &outcome.compiled {
        let Some(schema) = state.root_schema(&compiled.media_type) else {
            continue;
        };

        let music_directory = state.mpd_base();
        let Ok(parsed) = crate::ingest::parse_lock(
            &compiled.media_type,
            &compiled.lock_json,
            schema,
            music_directory.as_deref(),
        ) else {
            log::error!("Failed to parse lock for {}", compiled.dir.display());
            continue;
        };
        let Some(parsed) = parsed else {
            continue;
        };

        let eval = serde_json::from_str::<serde_json::Value>(&compiled.lock_json)
            .ok()
            .and_then(|lock| lock.get(&compiled.media_type).cloned())
            .and_then(|entity| engine.evaluate_logic(&compiled.media_type, &entity).ok())
            .unwrap_or_else(|| json!({}));

        if let Err(e) =
            logic.upsert_entity(parsed.entity, parsed.playables, parsed.images, eval)
        {
            log::error!("Failed to index {}: {e}", compiled.dir.display());
            continue;
        }

        // The index always takes the fresh data; only a changed lock is worth
        // telling clients about.
        let key = EntityKey {
            media_type: compiled.media_type.clone(),
            dir: compiled.dir.clone(),
        };
        if !outcome.changed.contains(&key) {
            continue;
        }

        let entry = per_type.entry(compiled.media_type.clone()).or_default();
        if let Some(dict) = logic.entity_dict(&compiled.media_type, &compiled.id) {
            entry.0.insert(compiled.id.clone(), dict.to_owned());
        }
    }

    logic.build_cache();
    per_type
}

fn shelves_for(logic: &LogicEngine, media_type: &str) -> HashMap<String, Vec<String>> {
    let mut shelves = HashMap::new();
    for (key, def) in &logic.manifest.shelves {
        if def.media != media_type {
            continue;
        }
        shelves.insert(
            key.clone(),
            logic.request_shelf_view(
                media_type,
                key,
                None,
                datamile_logic::SortOrder::Forward,
            ),
        );
    }
    shelves
}

#[allow(
    clippy::too_many_arguments,
    reason = "the pass mirrors the pending-update fields one to one"
)]
async fn run_update_pass(
    state: Arc<ServerState>,
    shared: Arc<MediaShared>,
    scope: UpdateScope,
    force: ForceMode,
    jobs: Option<usize>,
    event_txs: Vec<tokio::sync::mpsc::Sender<String>>,
) -> Result<PassResult> {
    // The journal always runs; each attached client adds one bounded stream.
    let sinks = CompositeSink::new(
        std::iter::once(Arc::new(LogSink) as Arc<dyn UpdateSink>)
            .chain(
                event_txs
                    .into_iter()
                    .map(|tx| Arc::new(StreamSink::new(tx)) as Arc<dyn UpdateSink>),
            )
            .collect(),
    );

    let shared_for_pass = Arc::clone(&shared);
    let outcome = tokio::task::spawn_blocking(move || {
        run_pass(&shared_for_pass, &scope, force, jobs, &sinks)
    })
    .await??;

    let media_state = shared
        .state
        .read()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .clone()
        .context("Media state is not loaded")?;

    let logic_arc = Arc::clone(&state.logic);
    let ingest_state = media_state.clone();
    let per_type = tokio::task::spawn_blocking(move || {
        let mut logic = logic_arc.blocking_write();
        let per_type = ingest_outcome(&ingest_state, &mut logic, &outcome);
        drop(logic);
        (per_type, outcome.result)
    })
    .await?;

    let (per_type, result) = per_type;

    for (media_type, (updated, removed)) in per_type {
        if updated.is_empty() && removed.is_empty() {
            continue;
        }
        let shelves = {
            let logic = state.logic.read().await;
            shelves_for(&logic, &media_type)
        };
        let _ = state.tx.send(
            json!({
                "type": "MEDIA_UPDATED",
                "media_type": media_type,
                "updated": updated,
                "removed": removed,
                "shelves": shelves
            })
            .to_string(),
        );
    }

    Ok(result)
}

/// The scope a request asks for: explicit entity keys win, then a path, then
/// everything. Entity dirs are canonicalized so they compare equal to the
/// entity index keys.
#[must_use]
pub fn scope_for_request(request: &UpdateRequest) -> UpdateScope {
    if !request.entities.is_empty() {
        let entities = request
            .entities
            .iter()
            .map(|key| EntityKey {
                media_type: key.media_type.clone(),
                dir: canonical_or_self(&key.dir),
            })
            .collect();
        return UpdateScope::Entities(entities);
    }

    request
        .path
        .as_deref()
        .map(datamile_fs::expand_path)
        .map_or_else(
            || UpdateScope::All {
                media_type: request.media_type.clone(),
            },
            |path| UpdateScope::Paths {
                paths: HashSet::from([canonical_or_self(&path)]),
                media_type: request.media_type.clone(),
            },
        )
}

pub async fn run_server_update(
    state: Arc<ServerState>,
    shared: Arc<MediaShared>,
    request: UpdateRequest,
    event_txs: Vec<tokio::sync::mpsc::Sender<String>>,
) -> Result<PassResult> {
    run_scope_update(
        state,
        shared,
        scope_for_request(&request),
        ForceMode::from(request.force),
        request.jobs,
        event_txs,
    )
    .await
}

/// Runs one scope through the single serialized pipeline. The watcher submits
/// here too, so it never compiles, writes state or broadcasts on its own.
pub async fn run_scope_update(
    state: Arc<ServerState>,
    shared: Arc<MediaShared>,
    mut scope: UpdateScope,
    mut force: ForceMode,
    mut jobs: Option<usize>,
    mut event_txs: Vec<tokio::sync::mpsc::Sender<String>>,
) -> Result<PassResult> {
    {
        let mut lock = shared
            .update_lock
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if lock.is_running {
            if let Some(pending) = lock.pending.as_mut() {
                pending.scope.merge(scope);
                pending.force |= force == ForceMode::Force;
                pending.jobs = jobs.or(pending.jobs);
                pending.event_txs.extend(event_txs);
            } else {
                lock.pending = Some(crate::deps::PendingUpdate {
                    scope,
                    force: force == ForceMode::Force,
                    jobs,
                    event_txs,
                });
            }
            return Ok(PassResult::default());
        }
        lock.is_running = true;
    }

    let mut last;
    loop {
        let result = run_update_pass(
            Arc::clone(&state),
            Arc::clone(&shared),
            scope.clone(),
            force,
            jobs,
            event_txs.clone(),
        )
        .await;

        last = result;

        let next = {
            let mut lock = shared
                .update_lock
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            lock.pending.take().map_or_else(
                || {
                    lock.is_running = false;
                    None
                },
                Some,
            )
        };

        let Some(next) = next else {
            break;
        };
        scope = next.scope;
        force = ForceMode::from(next.force);
        jobs = next.jobs;
        event_txs = next.event_txs;
    }

    last
}

/// Runs one pass in this process, for a CLI that holds the instance lock.
pub fn run_local(
    shared: &Arc<MediaShared>,
    path: Option<PathBuf>,
    media_type: Option<&str>,
    force: ForceMode,
    jobs: Option<usize>,
    sink: &dyn UpdateSink,
) -> Result<PassResult> {
    let media_type = media_type.map(str::to_string);
    let scope = match path {
        Some(path) => UpdateScope::Paths {
            paths: HashSet::from([path]),
            media_type,
        },
        None => UpdateScope::All { media_type },
    };

    run_pass(shared, &scope, force, jobs, sink).map(|outcome| outcome.result)
}

/// A path that no longer exists keeps its raw form, so a vanished target still
/// compares against the state entry it must remove.
pub(crate) fn canonical_or_self(path: &Path) -> PathBuf {
    path.canonicalize().unwrap_or_else(|_| path.to_path_buf())
}

/// Streams a pass from the daemon, rendering each event through `sink`.
///
/// An unterminated stream is an error: a dropped `Finished` must never turn a
/// failure into a success.
pub async fn delegate(
    port: u16,
    path: Option<PathBuf>,
    media_type: Option<String>,
    force: ForceMode,
    jobs: Option<usize>,
    sink: &dyn UpdateSink,
) -> Result<PassResult> {
    let body = json!({
        "path": path.map(|p| p.to_string_lossy().to_string()),
        "media_type": media_type,
        "force": force == ForceMode::Force,
        "jobs": jobs,
    });

    let mut response = local_client()
        .post(format!("http://127.0.0.1:{port}/api/update"))
        .json(&body)
        .send()
        .await
        .context("Failed to reach the running daemon")?;

    if !response.status().is_success() {
        let text = response.text().await.unwrap_or_default();
        anyhow::bail!("Daemon update failed: {text}");
    }

    let mut finished = None;
    let mut buffer = String::new();

    loop {
        match response.chunk().await {
            Ok(Some(chunk)) => buffer.push_str(&String::from_utf8_lossy(&chunk)),
            Ok(None) => break,
            Err(e) => return Err(e).context("Update stream broke"),
        }

        while let Some(position) = buffer.find('\n') {
            let line: String = buffer.drain(..=position).collect();
            if let Some(event) = parse_event(&line, sink) {
                finished = Some(event);
            }
        }
    }

    if let Some(event) = parse_event(&buffer, sink) {
        finished = Some(event);
    }

    finished.context("The update stream ended without a Finished event")
}

/// Every call here is loopback, so an `HTTP_PROXY` in the environment must not
/// route it.
fn local_client() -> reqwest::Client {
    reqwest::Client::builder()
        .no_proxy()
        .build()
        .unwrap_or_default()
}

fn parse_event(line: &str, sink: &dyn UpdateSink) -> Option<PassResult> {
    let line = line.trim();
    if line.is_empty() {
        return None;
    }

    let Ok(event) = serde_json::from_str::<UpdateEvent>(line) else {
        log::warn!("Ignoring an unreadable update line: {line}");
        return None;
    };
    sink.event(&event);

    match event {
        UpdateEvent::Finished {
            discovered,
            added,
            updated,
            unchanged,
            removed,
            failed,
        } => Some(PassResult {
            discovered,
            added,
            updated,
            unchanged,
            removed,
            failed,
        }),
        _ => None,
    }
}
