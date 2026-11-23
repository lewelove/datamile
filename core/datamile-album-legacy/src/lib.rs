pub mod backend;
pub mod compile;
pub mod compiler;
pub mod config;
mod deps;
pub mod error;
mod ingest;
mod inotify;
mod library;
pub mod lua;
mod lua_bridge;
pub mod resolvers;
pub mod scanner;
pub mod update;
pub mod x;

pub use backend::AlbumLegacyBackend;
pub use deps::{DependencyGraph, LegacyShared, UpdateScope, UpdateState};
