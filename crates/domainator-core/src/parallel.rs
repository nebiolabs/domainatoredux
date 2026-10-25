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

/// Run `f` on a pool of [`resolve_cpu`]`(cpu)` threads.
pub fn with_threads<R, F>(cpu: usize, f: F) -> R
where
    F: FnOnce() -> R + Send,
    R: Send,
{
    rayon::ThreadPoolBuilder::new()
        .num_threads(resolve_cpu(cpu))
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
}
