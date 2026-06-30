//! Thread-pool helper for the parallel tool paths. `cpu == 0` means "use all
//! available cores" (Rayon's global pool); any other value runs the closure on a
//! scoped pool with exactly that many threads, mirroring the Python suite's
//! `--cpu` convention.

/// Run `f`, parallelized over `cpu` threads (`0` = all cores / global pool).
pub fn with_threads<R, F>(cpu: usize, f: F) -> R
where
    F: FnOnce() -> R + Send,
    R: Send,
{
    if cpu == 0 {
        f()
    } else {
        rayon::ThreadPoolBuilder::new()
            .num_threads(cpu)
            .build()
            .expect("failed to build thread pool")
            .install(f)
    }
}
