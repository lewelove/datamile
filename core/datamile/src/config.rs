use anyhow::Result;
use libdatamile::lua::ResolvedConfig;

/// Loads `init.lua` and runs the album-level checks every command relies on,
/// including hot reloads, so no entry point can run with an unfiltered or
/// unsupported config.
pub fn load() -> Result<ResolvedConfig> {
    let mut config = ResolvedConfig::load()?;
    crate::compile::context::validate_config(&mut config)?;
    Ok(config)
}
