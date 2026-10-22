//! The `domainatoredux` binary. Everything is in the library (`src/lib.rs`), which the
//! GUI's worker mode runs too.

fn main() -> anyhow::Result<()> {
    match domainator_cli::run(std::env::args_os()) {
        Err(e) => match domainator_cli::clap_error(&e) {
            Some(clap_err) => clap_err.exit(),
            None => Err(e),
        },
        Ok(()) => Ok(()),
    }
}
