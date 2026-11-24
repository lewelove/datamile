pub mod backend;
pub mod compiler;
pub mod deps;
pub mod ingest;
pub mod lua_bridge;
pub mod schema;
pub mod state;
pub mod update;
pub mod watcher;

#[cfg(test)]
mod tests;

pub use backend::MediaEngineBackend;
pub use compiler::{CompiledRoot, compile_root, preview_root};
pub use deps::{DependencyGraph, MediaShared};
pub use ingest::{ParsedEntity, parse_lock};
pub use schema::{
    DiscoveredRoot, MediaManifest, MediaNodeSchema, MediaRootSchema, discover_roots,
    read_media_manifest, validate_media_schema,
};
pub use state::{MediaState, open_engine};
pub use update::{ForceMode, Verbosity};
