use color_eyre::eyre::{Result, WrapErr as _};
use log::LevelFilter;
use rsonpath_lib::query_rewrite;
use simple_logger::SimpleLogger;

/// Initialize logging for rewrite binaries (`-v` / `-vv`).
///
/// Global level stays at `Warn`; verbose flags only raise verbosity under
/// [`query_rewrite::LOG_TARGET`].
pub fn init(verbose: u8) -> Result<()> {
    logger(verbose, None)
        .init()
        .wrap_err("Logger configuration error.")
}

/// Like [`init`], but always enables `Info` on `binary_module`.
///
/// Untargeted `log::info!` in that binary is visible without `-v`.
/// `-v` / `-vv` raise the binary together with the rewrite pipeline.
pub fn init_for_binary(verbose: u8, binary_module: &str) -> Result<()> {
    logger(verbose, Some(binary_module))
        .init()
        .wrap_err("Logger configuration error.")
}

fn logger(verbose: u8, binary_module: Option<&str>) -> SimpleLogger {
    let mut logger = SimpleLogger::new().with_level(LevelFilter::Warn);

    let verbose_level = match verbose {
        0 => None,
        1 => Some(LevelFilter::Debug),
        _ => Some(LevelFilter::Trace),
    };

    if let Some(module) = binary_module {
        logger = logger.with_module_level(module, verbose_level.unwrap_or(LevelFilter::Info));
    }

    if let Some(level) = verbose_level {
        logger = logger.with_module_level(query_rewrite::LOG_TARGET, level);
    }

    logger
}
