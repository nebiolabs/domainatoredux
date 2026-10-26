//! `domainatoredux doctor`: which optional external programs this installation can
//! run (upstream `cli/doctor.py`, less its Python packages, which have no counterpart).

use domainator_core::tools::external::EXTERNAL_PROGRAMS;
use domainator_core::version::LONG_VERSION;

/// The report, one line per entry: each program, whether a tool would find it (and
/// where), the commands that need it, and how to install it.
pub fn report() -> Vec<String> {
    let mut lines = vec![
        format!(
            "Domainatoredux {LONG_VERSION}, on {} {}",
            std::env::consts::OS,
            std::env::consts::ARCH
        ),
        String::new(),
        "External programs:".to_string(),
    ];
    for program in EXTERNAL_PROGRAMS {
        let tried = program.find();
        let status = match program.locate() {
            Some(path) => format!("found ({})", path.display()),
            // An environment variable or bundled path that names no file.
            None if tried.components().count() > 1 => format!(
                "missing: {} does not exist; install: {}",
                tried.display(),
                program.install
            ),
            None => format!(
                "missing; install: {}; or set ${} to its path",
                program.install, program.env_vars[0]
            ),
        };
        lines.push(format!("  {:<9} {status}", program.name));
        lines.push(format!("  {:<9} needed by: {}", "", program.needed_by));
    }
    lines.push(String::new());
    lines.push(
        "The desktop app's External tools panel downloads all of them; see \
         docs/installation.md."
            .to_string(),
    );
    lines
}

pub fn run() {
    println!("{}", report().join("\n"));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_program_is_reported_with_its_users() {
        let report = report().join("\n");
        for program in EXTERNAL_PROGRAMS {
            assert!(
                report.contains(&format!("  {:<9} ", program.name)),
                "{}",
                program.name
            );
            assert!(report.contains(program.needed_by), "{}", program.name);
        }
    }
}
