//! The `domainatoredux` binary. Everything is in the library (`src/lib.rs`), which the
//! GUI's worker mode runs too.

fn main() {
    std::process::exit(domainator_cli::main_with(std::env::args_os()));
}
