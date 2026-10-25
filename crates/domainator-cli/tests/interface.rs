//! Rules every tool's interface follows, checked generically so a new tool or option
//! cannot skip them. The interface is the clap command; `describe`, `--config` and
//! `--print_config` are all read off it, so these hold for every one of them.

use std::ffi::OsString;

use domainator_config::manifest::{parameters, path_role};
use domainator_config::registry::{tool_info, TOOLS};
use domainator_config::types::type_info;

/// The dispatcher's own commands, which are not tools.
const BUILTINS: &[&str] = &["describe", "completion", "schema"];

fn with_big_stack(f: impl FnOnce() + Send + 'static) {
    // The CLI's `Command` is one large derive; building it overflows a test thread's
    // 2 MB stack in a debug build.
    std::thread::Builder::new()
        .stack_size(256 * 1024 * 1024)
        .spawn(f)
        .unwrap()
        .join()
        .unwrap_or_else(|e| std::panic::resume_unwind(e));
}

#[test]
fn every_subcommand_is_in_the_registry_and_back() {
    with_big_stack(|| {
        let cmd = domainator_cli::command();
        let subs: Vec<&str> = cmd.get_subcommands().map(|c| c.get_name()).collect();
        for s in &subs {
            assert!(
                BUILTINS.contains(s) || tool_info(s).is_some(),
                "subcommand {s} has no registry entry"
            );
        }
        for t in TOOLS {
            assert!(
                subs.contains(&t.name),
                "registry lists {} but the CLI has no such subcommand",
                t.name
            );
            let about = cmd
                .find_subcommand(t.name)
                .unwrap()
                .get_about()
                .map(|a| a.to_string());
            assert_eq!(
                about.as_deref(),
                Some(t.summary),
                "{}: --help summary",
                t.name
            );
        }
    });
}

#[test]
fn migrated_tools_follow_upstreams_naming_rules() {
    with_big_stack(|| {
        let cmd = domainator_cli::command();
        for info in TOOLS.iter().filter(|t| t.migrated) {
            let sub = cmd.find_subcommand(info.name).unwrap();
            for arg in sub.get_arguments() {
                let id = arg.get_id().as_str();
                if let Some(long) = arg.get_long() {
                    assert_eq!(long, id, "{}: --{long} is parameter {id}", info.name);
                }
            }
            for arg in parameters(sub) {
                let id = arg.get_id().as_str();
                assert_eq!(
                    path_role(id) == Some("output"),
                    id.ends_with("_out"),
                    "{}: {id}",
                    info.name
                );
            }
            for out in info
                .required_outputs
                .unwrap_or_default()
                .iter()
                .chain(info.no_stdout)
            {
                assert!(
                    parameters(sub).any(|a| a.get_id() == *out),
                    "{}: registry names output {out}, which the tool does not have",
                    info.name
                );
            }
        }
    });
}

/// Upstream's restricted float types refuse `nan` and `inf`; ours must too, on every
/// numeric option of a migrated tool.
#[test]
fn every_number_option_rejects_nan_and_inf() {
    with_big_stack(|| {
        let cmd = domainator_cli::command();
        for info in TOOLS.iter().filter(|t| t.migrated) {
            let sub = cmd.find_subcommand(info.name).unwrap();
            for arg in parameters(sub) {
                let ty = type_info(arg.get_value_parser()).json_type;
                if !matches!(ty, "number" | "integer") {
                    continue;
                }
                let long = arg.get_long().unwrap();
                for bad in ["nan", "inf"] {
                    let argv: Vec<OsString> = [
                        "domainatoredux",
                        info.name,
                        &format!("--{long}={bad}"),
                        "--print_config",
                    ]
                    .iter()
                    .map(OsString::from)
                    .collect();
                    assert!(
                        domainator_cli::run(argv).is_err(),
                        "{} --{long} accepted {bad}",
                        info.name
                    );
                }
            }
        }
    });
}

/// A config file of `--print_config`'s output gives the same `--print_config`.
#[test]
fn print_config_is_a_config_file() {
    with_big_stack(|| {
        let dir =
            std::env::temp_dir().join(format!("domainatoredux-interface-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let cmd = domainator_cli::command();
        for info in TOOLS.iter().filter(|t| t.migrated) {
            let sub = cmd.find_subcommand(info.name).unwrap();
            let m = sub
                .clone()
                .mut_args(|a| a.required(false))
                .try_get_matches_from([info.name])
                .unwrap();
            let yaml = domainator_config::config_file::print_config(sub, &m, info).unwrap();
            let path = dir.join(format!("{}.yml", info.name));
            std::fs::write(&path, &yaml).unwrap();
            let args = domainator_config::config_file::expand(
                sub,
                &[OsString::from("--config"), path.into_os_string()],
            )
            .unwrap();
            let again = sub
                .clone()
                .mut_args(|a| a.required(false))
                .try_get_matches_from(std::iter::once(OsString::from(info.name)).chain(args))
                .unwrap();
            assert_eq!(
                domainator_config::config_file::print_config(sub, &again, info).unwrap(),
                yaml,
                "{}",
                info.name
            );
        }
    });
}
