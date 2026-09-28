//! Tools-menu jobs run in a copy of this executable, started in a hidden worker
//! mode.
//!
//! `main.rs` calls [`maybe_run_worker`] before anything else. When the first
//! argument is [`WORKER_FLAG`], the rest of the command line is exactly a
//! `domainator` CLI command line (`domain-search --config <file>`), which is handed
//! to [`domainator_cli::run`] -- the same code the `domainator` binary runs -- and
//! the process exits without ever starting Tauri, GTK or a webview. There is still
//! only one binary to build, bundle and sign.
//!
//! A job in its own process is what makes Cancel real ([`Killer`] kills the whole
//! process tree, diamond and cd-hit included), keeps a crashing tool (an
//! out-of-memory abort, a stack overflow) from taking the app and its unsaved
//! editors down, and hands each job's peak memory back to the OS when it ends.
//!
//! **Lifeline.** A worker reads its stdin until end of file, then exits (killing
//! its process group on Unix, so a diamond it started goes too). The parent keeps
//! the worker's stdin pipe open for as long as it wants the job to run, so if the
//! GUI crashes or is killed the pipe closes and the worker stops. That works on
//! every platform, macOS included, which has no `PR_SET_PDEATHSIG`. On Windows the
//! Job Object's `KILL_ON_JOB_CLOSE` covers the same case from the other side. A
//! consequence: a worker started with stdin at end of file (`< /dev/null`) exits
//! at once. The mode is hidden and only the GUI starts it, so that is fine.

use std::ffi::{OsStr, OsString};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, ExitStatus, Stdio};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;

/// `argv[1]` of a worker. Everything after it is a CLI command line.
pub const WORKER_FLAG: &str = "--domainator-worker";

/// Exit code of a worker that stopped because its stdin closed (the GUI went
/// away) and that could not take its process group with it (it is not the group
/// leader, so it was not started by [`spawn`]).
pub const EXIT_ORPHANED: i32 = 3;

/// How much of each of a worker's stdout and stderr is kept in memory for the
/// job's message. All of it goes to the log file.
const TAIL_BYTES: usize = 16 * 1024;

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

// ---------------------------------------------------------------------------
// Worker side
// ---------------------------------------------------------------------------

/// Run as a worker if this process was started as one, returning its exit code.
///
/// `None` means "not a worker": start the GUI. Must stay the first thing `main`
/// does -- see `main.rs`.
pub fn maybe_run_worker() -> Option<i32> {
    let mut args = std::env::args_os().skip(1);
    if args.next().as_deref() != Some(OsStr::new(WORKER_FLAG)) {
        return None;
    }
    Some(run_worker(args.collect()))
}

fn run_worker(args: Vec<OsString>) -> i32 {
    #[cfg(debug_assertions)]
    if let Some(code) = test_command(&args) {
        return code;
    }
    watch_lifeline();
    let argv = std::iter::once(OsString::from("domainator"))
        .chain(args)
        .collect();
    match run_cli(argv) {
        Ok(()) => 0,
        Err(e) => match domainator_cli::clap_error(&e) {
            Some(clap_err) => {
                let _ = clap_err.print();
                clap_err.exit_code()
            }
            // What the `domainator` binary prints for an error (`main` returning
            // `Err`), so a job's message is the one the CLI would show.
            None => {
                eprintln!("Error: {e:?}");
                1
            }
        },
    }
}

/// Stack for the thread [`run_cli`] runs the CLI on. Only reserved, not committed.
const CLI_STACK_BYTES: usize = 256 * 1024 * 1024;

/// [`domainator_cli::run`] on a thread with a large stack.
///
/// The CLI's dispatch is one very large function, and an unoptimized build gives
/// it a stack frame that overflows a 2 MB thread (Rust's default for spawned
/// threads, and tokio's for its blocking pool) as well as Windows' 1 MB main
/// thread. Every path that runs the CLI inside the GUI's executable -- a worker,
/// the in-process escape hatch, the tests -- goes through here. A panic in the
/// tool comes back as `Err`, with the panic message already on stderr.
pub fn run_cli(argv: Vec<OsString>) -> anyhow::Result<()> {
    std::thread::Builder::new()
        .name("domainator-cli".into())
        .stack_size(CLI_STACK_BYTES)
        .spawn(move || domainator_cli::run(argv))
        .map_err(|e| anyhow::anyhow!("starting the tool's thread: {e}"))?
        .join()
        .unwrap_or_else(|panic| {
            let what = panic
                .downcast_ref::<&str>()
                .map(|s| s.to_string())
                .or_else(|| panic.downcast_ref::<String>().cloned())
                .unwrap_or_default();
            Err(anyhow::anyhow!("the tool crashed: {what}"))
        })
}

/// Exit when stdin reaches end of file -- the parent has gone. See the module note.
fn watch_lifeline() {
    std::thread::spawn(|| {
        let mut stdin = std::io::stdin();
        let mut buf = [0u8; 64];
        loop {
            match stdin.read(&mut buf) {
                Ok(0) => break,
                Ok(_) => {}
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
                Err(_) => break,
            }
        }
        orphaned_exit()
    });
}

fn orphaned_exit() -> ! {
    #[cfg(unix)]
    // SAFETY: plain libc calls with no pointers. The group is killed only when this
    // process leads it, which is what `spawn`'s `process_group(0)` sets up: a
    // worker run any other way (by hand, from a test harness) must not signal a
    // group it merely belongs to.
    unsafe {
        if libc::getpgrp() == libc::getpid() {
            libc::kill(0, libc::SIGKILL);
        }
    }
    std::process::exit(EXIT_ORPHANED)
}

/// Debug-build-only commands for the worker tests (`tests/worker.rs`). Compiled
/// out of release builds, so a release worker only ever runs the CLI.
///
/// * `__sleep <secs>` -- sleep, then exit 0.
/// * `__sleep-with-child <secs>` -- start a grandchild that sleeps too, print
///   `child <pid>` on stdout, then sleep. The grandchild does not watch its stdin,
///   like a real diamond or cd-hit, so only a tree kill can stop it early.
#[cfg(debug_assertions)]
fn test_command(args: &[OsString]) -> Option<i32> {
    let name = args.first()?.to_str()?;
    let secs: f64 = args
        .get(1)
        .and_then(|s| s.to_str())
        .and_then(|s| s.parse().ok())
        .unwrap_or(0.0);
    let sleep = || std::thread::sleep(std::time::Duration::from_secs_f64(secs));
    match name {
        "__sleep" => {
            watch_lifeline();
            sleep();
            Some(0)
        }
        "__sleep-unwatched" => {
            sleep();
            Some(0)
        }
        "__sleep-with-child" => {
            watch_lifeline();
            let exe = std::env::current_exe().ok()?;
            let child = Command::new(exe)
                .arg(WORKER_FLAG)
                .arg("__sleep-unwatched")
                .arg(secs.to_string())
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .ok()?;
            println!("child {}", child.id());
            sleep();
            Some(0)
        }
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// Parent side
// ---------------------------------------------------------------------------

/// The executable to start a worker from: this one.
///
/// On Linux `/proc/self/exe`, which still names this process's own image after an
/// update has replaced the installed file (and an AppImage's mount lives as long
/// as this process does).
pub fn worker_exe() -> std::io::Result<PathBuf> {
    #[cfg(target_os = "linux")]
    {
        let p = PathBuf::from("/proc/self/exe");
        if p.exists() {
            return Ok(p);
        }
    }
    std::env::current_exe()
}

/// Start a worker of this executable running `args` (a CLI command line without
/// the program name). See [`spawn_exe`].
pub fn spawn(args: &[OsString], cwd: Option<&Path>, log: Option<&Path>) -> std::io::Result<Worker> {
    spawn_exe(&worker_exe()?, args, cwd, log)
}

/// Start `exe --domainator-worker <args>` in its own process tree, with stdin
/// piped as the lifeline and stdout/stderr copied to `log` (when given) and kept
/// in memory, last `TAIL_BYTES` of each.
pub fn spawn_exe(
    exe: &Path,
    args: &[OsString],
    cwd: Option<&Path>,
    log: Option<&Path>,
) -> std::io::Result<Worker> {
    let mut cmd = Command::new(exe);
    cmd.arg(WORKER_FLAG)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if let Some(dir) = cwd {
        cmd.current_dir(dir);
    }
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        // Its own process group, which is what `Killer` signals.
        cmd.process_group(0);
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        // A release build is a windowed app and has no console anyway; this also
        // keeps a debug build's worker from opening one.
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    let log_file = match log {
        Some(p) => Some(std::fs::File::create(p)?),
        None => None,
    };
    let mut child = cmd.spawn()?;
    let killer = match Killer::for_child(&child) {
        Ok(k) => k,
        Err(e) => {
            let _ = child.kill();
            let _ = child.wait();
            return Err(e);
        }
    };
    let log_file = Arc::new(Mutex::new(log_file));
    let stdout = Arc::new(Mutex::new(Vec::new()));
    let stderr = Arc::new(Mutex::new(Vec::new()));
    let mut readers = Vec::new();
    if let Some(out) = child.stdout.take() {
        readers.push(copy_stream(out, log_file.clone(), stdout.clone()));
    }
    if let Some(err) = child.stderr.take() {
        readers.push(copy_stream(err, log_file.clone(), stderr.clone()));
    }
    let lifeline = child.stdin.take();
    Ok(Worker {
        child: Some(child),
        lifeline,
        killer,
        readers,
        stdout,
        stderr,
    })
}

fn copy_stream(
    mut from: impl Read + Send + 'static,
    log: Arc<Mutex<Option<std::fs::File>>>,
    tail: Arc<Mutex<Vec<u8>>>,
) -> JoinHandle<()> {
    std::thread::spawn(move || {
        let mut buf = [0u8; 8192];
        loop {
            let n = match from.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => n,
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(_) => break,
            };
            if let Some(f) = lock(&log).as_mut() {
                let _ = std::io::Write::write_all(f, &buf[..n]);
            }
            let mut t = lock(&tail);
            t.extend_from_slice(&buf[..n]);
            if t.len() > 2 * TAIL_BYTES {
                let cut = t.len() - TAIL_BYTES;
                t.drain(..cut);
            }
        }
    })
}

/// A running worker. Dropping it without [`Worker::wait`] kills its tree.
pub struct Worker {
    child: Option<Child>,
    /// Held open for as long as the job should run. See the module note.
    lifeline: Option<ChildStdin>,
    killer: Killer,
    readers: Vec<JoinHandle<()>>,
    stdout: Arc<Mutex<Vec<u8>>>,
    stderr: Arc<Mutex<Vec<u8>>>,
}

/// How a worker ended.
pub struct WorkerExit {
    pub status: ExitStatus,
    /// The last `TAIL_BYTES` of its stdout, lossily decoded.
    pub stdout: String,
    pub stderr: String,
}

impl Worker {
    pub fn id(&self) -> u32 {
        self.child.as_ref().map(Child::id).unwrap_or(0)
    }

    /// A handle that kills this worker's whole tree from any thread.
    pub fn killer(&self) -> Killer {
        self.killer.clone()
    }

    /// The stdout captured so far (its last `TAIL_BYTES`).
    pub fn stdout_so_far(&self) -> String {
        String::from_utf8_lossy(&lock(&self.stdout)).into_owned()
    }

    /// Block until the worker exits, then clean up anything it left running.
    pub fn wait(mut self) -> std::io::Result<WorkerExit> {
        let status = self.child.take().expect("waited once").wait()?;
        // A grandchild the worker left behind (it crashed, or was killed alone)
        // still holds the output pipes open, so the readers would never finish.
        self.killer.finish();
        for r in self.readers.drain(..) {
            let _ = r.join();
        }
        self.lifeline = None;
        Ok(WorkerExit {
            status,
            stdout: String::from_utf8_lossy(&lock(&self.stdout)).into_owned(),
            stderr: String::from_utf8_lossy(&lock(&self.stderr)).into_owned(),
        })
    }
}

impl Drop for Worker {
    fn drop(&mut self) {
        if let Some(mut child) = self.child.take() {
            self.killer.kill();
            let _ = child.wait();
            self.killer.finish();
        }
    }
}

/// Kills a worker and everything it started: its process group on Unix, its Job
/// Object on Windows. Cloneable and usable from any thread, so Cancel can reach a
/// job whose thread is blocked in [`Worker::wait`].
#[derive(Clone)]
pub struct Killer(Arc<KillerInner>);

struct KillerInner {
    /// False once the worker has been reaped and its group cleaned up. After that
    /// its pid (and so the group id) may belong to an unrelated process.
    alive: Mutex<bool>,
    #[cfg(unix)]
    pgid: libc::pid_t,
    #[cfg(windows)]
    job: win::JobHandle,
}

impl Killer {
    #[cfg(unix)]
    fn for_child(child: &Child) -> std::io::Result<Killer> {
        let pgid = libc::pid_t::try_from(child.id())
            .map_err(|_| std::io::Error::other("worker pid out of range"))?;
        Ok(Killer(Arc::new(KillerInner {
            alive: Mutex::new(true),
            pgid,
        })))
    }

    #[cfg(windows)]
    fn for_child(child: &Child) -> std::io::Result<Killer> {
        use std::os::windows::io::AsRawHandle;
        let job = win::JobHandle::new()?;
        job.assign(child.as_raw_handle())?;
        Ok(Killer(Arc::new(KillerInner {
            alive: Mutex::new(true),
            job,
        })))
    }

    #[cfg(not(any(unix, windows)))]
    fn for_child(_child: &Child) -> std::io::Result<Killer> {
        Err(std::io::Error::other(
            "worker processes are not supported on this platform",
        ))
    }

    /// Kill the whole tree now. A no-op once the worker has been reaped.
    pub fn kill(&self) {
        let alive = lock(&self.0.alive);
        if *alive {
            self.0.kill_tree();
        }
    }

    /// The worker has been reaped: kill whatever of its tree is left, and make
    /// every later [`Killer::kill`] a no-op.
    fn finish(&self) {
        let mut alive = lock(&self.0.alive);
        if *alive {
            self.0.kill_tree();
            *alive = false;
        }
    }
}

impl KillerInner {
    fn kill_tree(&self) {
        #[cfg(unix)]
        // SAFETY: no pointers. `alive` (held by the caller) guarantees the group
        // is still this worker's.
        unsafe {
            libc::killpg(self.pgid, libc::SIGKILL);
        }
        #[cfg(windows)]
        self.job.terminate();
    }
}

#[cfg(windows)]
mod win {
    use windows_sys::Win32::Foundation::{CloseHandle, HANDLE};
    use windows_sys::Win32::System::JobObjects::{
        AssignProcessToJobObject, CreateJobObjectW, JobObjectExtendedLimitInformation,
        SetInformationJobObject, TerminateJobObject, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
        JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
    };

    /// A Job Object with `KILL_ON_JOB_CLOSE`: closing it -- explicitly, or because
    /// the GUI died -- kills every process in it. A worker's children join its job
    /// automatically.
    pub struct JobHandle(HANDLE);

    // SAFETY: a job object handle is a kernel handle, usable from any thread.
    unsafe impl Send for JobHandle {}
    unsafe impl Sync for JobHandle {}

    impl JobHandle {
        pub fn new() -> std::io::Result<Self> {
            // SAFETY: null attributes and name are documented as valid; the info
            // struct is plain data, zeroed and then given the one flag.
            unsafe {
                let job = CreateJobObjectW(std::ptr::null(), std::ptr::null());
                if job.is_null() {
                    return Err(std::io::Error::last_os_error());
                }
                let handle = JobHandle(job);
                let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = std::mem::zeroed();
                info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
                let ok = SetInformationJobObject(
                    job,
                    JobObjectExtendedLimitInformation,
                    &info as *const _ as *const core::ffi::c_void,
                    size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
                );
                if ok == 0 {
                    return Err(std::io::Error::last_os_error());
                }
                Ok(handle)
            }
        }

        pub fn assign(&self, process: std::os::windows::io::RawHandle) -> std::io::Result<()> {
            // SAFETY: both handles are live; the process handle is the child's own.
            if unsafe { AssignProcessToJobObject(self.0, process as HANDLE) } == 0 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        }

        pub fn terminate(&self) {
            // SAFETY: a live job handle.
            unsafe {
                TerminateJobObject(self.0, 1);
            }
        }
    }

    impl Drop for JobHandle {
        fn drop(&mut self) {
            // SAFETY: closed exactly once.
            unsafe {
                CloseHandle(self.0);
            }
        }
    }
}
