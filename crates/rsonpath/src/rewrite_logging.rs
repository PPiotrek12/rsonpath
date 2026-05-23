use color_eyre::eyre::{Result, WrapErr as _};
use log::LevelFilter;
use rsonpath_lib::query_rewrite;
use simple_logger::SimpleLogger;

/// `log` target for rewrite CLI wrappers in [`crate::rewrite_tooling`].
pub const TOOLING_LOG_TARGET: &str = concat!("rsonpath_lib::query_rewrite", "::tooling");

/// Initialize logging for rewrite tooling binaries (`-v` / `-vv`).
///
/// Global level stays at `Warn`; verbose flags only raise verbosity under
/// [`query_rewrite::LOG_TARGET`] (library pipeline + tooling wrappers).
pub fn init(verbose: u8) -> Result<()> {
    let mut logger = SimpleLogger::new().with_level(LevelFilter::Warn);

    if verbose > 0 {
        let rewrite_level = if verbose == 1 {
            LevelFilter::Debug
        } else {
            LevelFilter::Trace
        };
        logger = logger.with_module_level(query_rewrite::LOG_TARGET, rewrite_level);
    }

    logger.init().wrap_err("Logger configuration error.")
}
