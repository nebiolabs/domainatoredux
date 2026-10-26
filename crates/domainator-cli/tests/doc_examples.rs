//! Every `domainatoredux` command in the docs' code blocks parses. The pages are
//! written by hand (each tool's purpose, the flags that matter, a worked example), so
//! nothing regenerates them when an option is renamed or a choice goes away; this
//! test is what notices. It checks flag names, choices and value types with the real
//! clap command, not whether the example's files exist.

use std::path::{Path, PathBuf};

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

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// The pages whose examples are checked: the book and the front page.
fn pages() -> Vec<PathBuf> {
    let root = repo_root();
    let mut pages: Vec<PathBuf> = std::fs::read_dir(root.join("docs"))
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|e| e == "md"))
        .collect();
    pages.push(root.join("README.md"));
    pages.sort();
    pages
}

/// The shell lines inside a page's ```bash and bare ``` blocks, with backslash
/// continuations joined, each paired with the line number it starts on.
fn shell_lines(text: &str) -> Vec<(usize, String)> {
    let mut out = Vec::new();
    let mut in_block = false;
    let mut pending: Option<(usize, String)> = None;
    for (i, line) in text.lines().enumerate() {
        let trimmed = line.trim_start();
        if let Some(lang) = trimmed.strip_prefix("```") {
            in_block = !in_block && matches!(lang.trim(), "" | "bash" | "sh");
            continue;
        }
        if !in_block {
            continue;
        }
        let (start, mut joined) = pending.take().unwrap_or((i + 1, String::new()));
        match line.trim_end().strip_suffix('\\') {
            Some(head) => {
                joined.push_str(head);
                joined.push(' ');
                pending = Some((start, joined));
            }
            None => {
                joined.push_str(line);
                out.push((start, joined));
            }
        }
    }
    out
}

/// Whether a line is a counterexample, commented `# error`, which must not parse.
fn is_counterexample(line: &str) -> bool {
    line.split_once(" #")
        .is_some_and(|(_, comment)| comment.trim_start().starts_with("error"))
}

/// The `domainatoredux` command lines in one shell line: split at pipes and `&&`,
/// with redirections, comments and anything before the program name dropped. A
/// synopsis (`domainatoredux <tool> [options]`) is not a command and is skipped.
fn commands(line: &str) -> Vec<Vec<String>> {
    let Some(words) = shlex::split(line) else {
        return Vec::new();
    };
    let mut groups: Vec<Vec<String>> = vec![Vec::new()];
    let mut words = words.into_iter();
    while let Some(w) = words.next() {
        if w.starts_with('#') {
            break;
        }
        if matches!(w.as_str(), "|" | "&&" | "||" | ";") {
            groups.push(Vec::new());
        } else if matches!(w.as_str(), ">" | ">>" | "<" | "2>" | "2>>") {
            words.next();
        } else if !(w.starts_with('>') || w.starts_with("2>")) {
            groups.last_mut().unwrap().push(w);
        }
    }
    groups
        .into_iter()
        .filter_map(|g| {
            let at = g.iter().position(|w| w == "domainatoredux")?;
            let argv = g[at..].to_vec();
            let synopsis = argv.iter().any(|w| {
                (w.starts_with('<') && w.ends_with('>'))
                    || (w.starts_with('[') && w.ends_with(']'))
                    || w == "..."
                    || w == "…"
            });
            (!synopsis).then_some(argv)
        })
        .collect()
}

#[test]
fn every_documented_command_parses() {
    with_big_stack(|| {
        let cmd = domainator_cli::command();
        let mut checked = 0;
        let mut failures = Vec::new();
        for page in pages() {
            let text = std::fs::read_to_string(&page).unwrap();
            let name = page.file_name().unwrap().to_string_lossy().into_owned();
            for (line_no, line) in shell_lines(&text) {
                let counterexample = is_counterexample(&line);
                for argv in commands(&line) {
                    let mut cmd = cmd.clone();
                    // A config file supplies options the line leaves out, so the
                    // line alone need not have every required one.
                    if let Some(tool) = argv.get(1) {
                        if argv
                            .iter()
                            .any(|w| w == "--config" || w.starts_with("--config="))
                        {
                            cmd = cmd.mut_subcommand(tool, |c| c.mut_args(|a| a.required(false)));
                        }
                    }
                    checked += 1;
                    let parsed = cmd.try_get_matches_from(&argv);
                    if counterexample {
                        if parsed.is_ok() {
                            failures.push(format!(
                                "{name}:{line_no}: {} parses, but is commented `# error`",
                                argv.join(" ")
                            ));
                        }
                    } else if let Err(e) = parsed {
                        if !matches!(
                            e.kind(),
                            clap::error::ErrorKind::DisplayHelp
                                | clap::error::ErrorKind::DisplayVersion
                                | clap::error::ErrorKind::DisplayHelpOnMissingArgumentOrSubcommand
                        ) {
                            let msg = e.to_string();
                            let first = msg.lines().next().unwrap_or_default();
                            failures
                                .push(format!("{name}:{line_no}: {}\n    {first}", argv.join(" ")));
                        }
                    }
                }
            }
        }
        assert!(checked > 50, "only {checked} documented commands found");
        assert!(
            failures.is_empty(),
            "{} documented command(s) do not parse:\n{}",
            failures.len(),
            failures.join("\n")
        );
    });
}

#[test]
fn shell_lines_are_split_into_commands() {
    let text = "```bash\ndomainatoredux a -i x \\\n  -o - | domainatoredux b -i - > out.tsv # note\n```\n```yaml\ndomainatoredux c\n```\n";
    let lines = shell_lines(text);
    assert_eq!(lines.len(), 1);
    assert_eq!(
        commands(&lines[0].1),
        vec![
            vec!["domainatoredux", "a", "-i", "x", "-o", "-"],
            vec!["domainatoredux", "b", "-i", "-"],
        ]
    );
    assert!(commands("domainatoredux <tool> [options]").is_empty());
    assert!(is_counterexample("domainatoredux --cpu 8 x   # error"));
    assert!(!is_counterexample("domainatoredux x   # fine"));
}
