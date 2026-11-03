//! Thread-pool helper for the parallel tool paths, following the Python suite's
//! `--cpu` convention: `0` means one thread per physical core available to this
//! process ([`resolve_cpu`]), any other value exactly that many threads.

/// The thread count for a `--cpu` value (upstream `_mp.resolve_cpu`): `cpu` itself,
/// or for `0` the physical cores, capped by the cores this process may run on.
pub fn resolve_cpu(cpu: usize) -> usize {
    if cpu > 0 {
        return cpu;
    }
    let available = std::thread::available_parallelism().map_or(1, |n| n.get());
    available.min(num_cpus::get_physical()).max(1)
}

/// The size [`init_global_pool`] gave rayon's global pool, if it built it.
static GLOBAL_THREADS: std::sync::OnceLock<usize> = std::sync::OnceLock::new();

/// Size rayon's global pool for a run's `--cpu`, before any parallel work starts.
///
/// Parallel code outside [`with_threads`] (reading an `.hmm`, for one) runs on the
/// global pool, which rayon would otherwise size at one thread per *logical* CPU,
/// whatever `--cpu` says. Only the first call in a process can size it; it returns
/// whether this one did (`false` once the global pool exists).
pub fn init_global_pool(cpu: usize) -> bool {
    let threads = resolve_cpu(cpu);
    let built = rayon::ThreadPoolBuilder::new()
        .num_threads(threads)
        .build_global()
        .is_ok();
    if built {
        let _ = GLOBAL_THREADS.set(threads);
    }
    built
}

/// Run `f` on a pool of [`resolve_cpu`]`(cpu)` threads.
///
/// Called from inside a rayon pool, `f` runs right there and `cpu` is ignored: the
/// enclosing pool already sets the parallelism (`domain_search` runs `domainate` on
/// each partition this way). Building a pool there would be worse than wasteful:
/// a worker that `install`s into another pool keeps stealing jobs from its own
/// while it waits, so every outer job could be started at once, each with a pool of
/// its own.
///
/// When [`init_global_pool`] already sized the global pool to the same count, `f`
/// also runs on the calling thread, its parallel work going to that pool, rather
/// than beside a second pool of the same size.
pub fn with_threads<R, F>(cpu: usize, f: F) -> R
where
    F: FnOnce() -> R + Send,
    R: Send,
{
    let threads = resolve_cpu(cpu);
    if rayon::current_thread_index().is_some() || GLOBAL_THREADS.get() == Some(&threads) {
        return f();
    }
    rayon::ThreadPoolBuilder::new()
        .num_threads(threads)
        .build()
        .expect("failed to build thread pool")
        .install(f)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zero_means_the_physical_cores_this_process_may_use() {
        assert_eq!(resolve_cpu(3), 3);
        let all = resolve_cpu(0);
        assert!(all >= 1);
        assert!(all <= std::thread::available_parallelism().unwrap().get());
        assert!(all <= num_cpus::get_physical());
        assert_eq!(with_threads(2, rayon::current_num_threads), 2);
        assert_eq!(with_threads(0, rayon::current_num_threads), all);
    }

    /// `domain_search` runs `domainate`, which calls `with_threads` itself, on every
    /// partition. A worker that `install`s into a *different* pool keeps stealing work
    /// from its own while it waits, so a fresh inner pool per call let each outer
    /// worker start partition after partition: nearly every partition in flight at
    /// once, each holding its records and a pool of `resolve_cpu(0)` threads, until
    /// the process ran out of mappings.
    #[test]
    fn a_nested_call_runs_in_the_enclosing_pool() {
        use rayon::prelude::*;
        use std::sync::atomic::{AtomicUsize, Ordering};

        let in_flight = AtomicUsize::new(0);
        let peak = AtomicUsize::new(0);
        with_threads(2, || {
            (0..64).into_par_iter().for_each(|_| {
                let now = in_flight.fetch_add(1, Ordering::SeqCst) + 1;
                peak.fetch_max(now, Ordering::SeqCst);
                with_threads(0, || {
                    std::thread::sleep(std::time::Duration::from_millis(2))
                });
                in_flight.fetch_sub(1, Ordering::SeqCst);
            });
        });
        assert!(
            peak.load(Ordering::SeqCst) <= 2,
            "peak {}",
            peak.load(Ordering::SeqCst)
        );

        assert_eq!(
            with_threads(2, || with_threads(0, rayon::current_num_threads)),
            2
        );
    }
}
