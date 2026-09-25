//! Worker-mode tests against the real `domainator-gui` binary (see `src/worker.rs`).
//!
//! Tools-menu jobs run in a copy of the GUI executable started with
//! `--domainator-worker`. Nothing but these tests exercises that path end to end:
//! it has no Python counterpart for the parity sweep to compare against. The
//! `__sleep*` worker commands they use exist only in debug builds, which is what
//! `cargo test` builds.

use std::ffi::OsString;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use domainator_gui::worker::{self, WORKER_FLAG};

const BIN: &str = env!("CARGO_BIN_EXE_domainator-gui");

fn scratch(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("domainator-gui-worker-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn args(a: &[&str]) -> Vec<OsString> {
    a.iter().map(OsString::from).collect()
}

/// Poll `f` until it returns `Some`, or panic after `secs`.
fn within<T>(secs: u64, what: &str, mut f: impl FnMut() -> Option<T>) -> T {
    let deadline = Instant::now() + Duration::from_secs(secs);
    loop {
        if let Some(v) = f() {
            return v;
        }
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// Whether a process is still running. A zombie (dead, not yet reaped by
/// whoever inherited it) counts as gone.
#[cfg(unix)]
fn alive(pid: i32) -> bool {
    // SAFETY: signal 0 only checks that the pid exists.
    if unsafe { libc::kill(pid, 0) } != 0 {
        return false;
    }
    match std::fs::read_to_string(format!("/proc/{pid}/stat")) {
        // "<pid> (<comm>) <state> ..." -- the state follows the last ')'.
        Ok(stat) => stat.rsplit_once(')').is_none_or(|(_, rest)| !rest.trim_start().starts_with('Z')),
        Err(_) => true, // no procfs (macOS): kill(0) is all there is
    }
}

/// The worker check must come before anything in `main` that starts Tauri: a
/// worker that initialized GTK/WebKit would need a display (breaking headless CI
/// and every job on a server) and could open a window.
#[test]
fn worker_mode_never_starts_tauri() {
    let mut child = Command::new(BIN)
        .args([WORKER_FLAG, "__sleep", "0"])
        .env_remove("DISPLAY")
        .env_remove("WAYLAND_DISPLAY")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let status = within(30, "the worker to exit", || child.try_wait().unwrap());
    let mut err = String::new();
    child.stderr.take().unwrap().read_to_string(&mut err).unwrap();
    assert!(status.success(), "worker exited {status:?}: {err}");
}

/// A worker runs the CLI: its exit code and stderr are the CLI's.
#[test]
fn a_worker_reports_cli_errors_like_the_cli() {
    let w = worker::spawn_exe(Path::new(BIN), &args(&["domainate", "-i", "/nonexistent.gb", "-r", "x.hmm", "-o", "o.gb"]), None, None)
        .unwrap();
    let exit = w.wait().unwrap();
    assert_eq!(exit.status.code(), Some(1));
    assert!(exit.stderr.starts_with("Error: "), "{}", exit.stderr);

    // A command line clap rejects: its usage error, and its exit code.
    let exit = worker::spawn_exe(Path::new(BIN), &args(&["no-such-tool"]), None, None).unwrap().wait().unwrap();
    assert_eq!(exit.status.code(), Some(2), "{}", exit.stderr);
}

/// Cancel must stop what the tool started too -- an orphaned diamond or cd-hit
/// would keep a core busy and write into a file the cleanup already removed.
#[cfg(unix)]
#[test]
fn cancel_kills_the_whole_tree() {
    let w = worker::spawn_exe(Path::new(BIN), &args(&["__sleep-with-child", "60"]), None, None).unwrap();
    let grandchild: i32 = within(30, "the grandchild's pid", || {
        w.stdout_so_far().lines().find_map(|l| l.strip_prefix("child ")?.trim().parse().ok())
    });
    assert!(alive(grandchild));
    let started = Instant::now();
    w.killer().kill();
    let exit = w.wait().unwrap();
    assert!(!exit.status.success());
    assert!(started.elapsed() < Duration::from_secs(10), "the kill did not stop the worker promptly");
    within(10, "the grandchild to die", || (!alive(grandchild)).then_some(()));
}

/// If the GUI dies, the pipe it held as the worker's stdin closes, and the worker
/// (with its process group) must stop rather than run on unowned.
#[cfg(unix)]
#[test]
fn a_worker_exits_when_its_stdin_closes() {
    use std::os::unix::process::CommandExt;
    // Started exactly as `worker::spawn` starts a worker: its own process group.
    let mut child = Command::new(BIN)
        .args([WORKER_FLAG, "__sleep-with-child", "60"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .process_group(0)
        .spawn()
        .unwrap();
    let mut line = Vec::new();
    let mut out = child.stdout.take().unwrap();
    let mut byte = [0u8; 1];
    while out.read(&mut byte).unwrap() == 1 && byte[0] != b'\n' {
        line.push(byte[0]);
    }
    let grandchild: i32 = String::from_utf8(line).unwrap().strip_prefix("child ").unwrap().trim().parse().unwrap();

    drop(child.stdin.take()); // the "GUI" goes away
    within(10, "the worker to exit", || child.try_wait().unwrap());
    within(10, "the grandchild to die", || (!alive(grandchild)).then_some(()));
}

/// Windows has no process groups; the worker's Job Object is what reaches the
/// grandchild. Only the direct child is checked here -- the grandchild is on the
/// Windows smoke list (README.md).
#[cfg(windows)]
#[test]
fn cancel_kills_the_worker() {
    let w = worker::spawn_exe(Path::new(BIN), &args(&["__sleep", "60"]), None, None).unwrap();
    let started = Instant::now();
    w.killer().kill();
    let exit = w.wait().unwrap();
    assert!(!exit.status.success());
    assert!(started.elapsed() < Duration::from_secs(10));
}

/// Everything a job's output goes to its log file too, which is where the full
/// error lands when the job message only has room for a line.
#[test]
fn a_workers_output_is_logged() {
    let d = scratch("log");
    let log = d.join("worker.log");
    let w = worker::spawn_exe(Path::new(BIN), &args(&["domainate", "-i", "/nonexistent.gb", "-r", "x", "-o", "o"]), None, Some(&log))
        .unwrap();
    let exit = w.wait().unwrap();
    let logged = std::fs::read_to_string(&log).unwrap();
    assert!(!logged.is_empty() && logged.contains(exit.stderr.trim()), "{logged:?}");
}

/// A GUI job and a CLI run must produce the same file: the worker is meant to be
/// the CLI, not a second implementation of it. A small `domainate` through a
/// worker with a `--config` file, as the GUI runs it, against the same run
/// through `domainator_cli::run` with flags.
#[test]
fn a_worker_run_matches_the_cli() {
    let repo = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../..");
    let data = repo.join("test/data");
    let d = scratch("e2e");
    let (via_worker, via_cli) = (d.join("worker.gb"), d.join("cli.gb"));
    let config = d.join("worker-config.json");
    let params = serde_json::json!({
        "input": data.join("FeSOD_20.fasta"),
        "references": [data.join("FeSOD_pfam.hmm")],
        "output": via_worker,
        "cpu": 2,
    });
    std::fs::write(&config, serde_json::to_vec(&params).unwrap()).unwrap();

    let exit = worker::spawn_exe(
        Path::new(BIN),
        &[OsString::from("domainate"), "--config".into(), config.clone().into_os_string()],
        Some(&d),
        None,
    )
    .unwrap()
    .wait()
    .unwrap();
    assert!(exit.status.success(), "{}", exit.stderr);
    assert!(exit.stdout.contains("wrote 20 records"), "{}", exit.stdout);

    let cli: Vec<OsString> = vec![
        "domainator".into(),
        "domainate".into(),
        "-i".into(),
        data.join("FeSOD_20.fasta").into(),
        "-r".into(),
        data.join("FeSOD_pfam.hmm").into(),
        "-o".into(),
        via_cli.clone().into(),
        "--cpu".into(),
        "2".into(),
    ];
    worker::run_cli(cli).unwrap();

    // The LOCUS line carries today's date (UTC); two runs straddling midnight
    // would differ there and nowhere else.
    let body = |p: &Path| -> Vec<String> {
        std::fs::read_to_string(p).unwrap().lines().filter(|l| !l.starts_with("LOCUS")).map(String::from).collect()
    };
    let (a, b) = (body(&via_worker), body(&via_cli));
    assert!(!a.is_empty());
    assert!(a == b, "the worker's output differs from the CLI's");
}
