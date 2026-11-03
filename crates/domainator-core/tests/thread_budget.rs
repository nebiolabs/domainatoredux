//! A run's `--cpu` bounds its threads: the CLI sizes rayon's global pool with
//! `init_global_pool`, and nothing after that builds another pool of the same size,
//! nests one, or sends parallel work to a pool of rayon's default size.
//!
//! Its own test binary, because the global pool can be sized only once per process.

use domainator_core::parallel::{init_global_pool, with_threads};
use rayon::prelude::*;

#[test]
fn the_sized_global_pool_is_the_only_pool() {
    assert!(init_global_pool(2));
    assert!(
        !init_global_pool(3),
        "the global pool can be sized only once"
    );

    // The same count runs on the caller, with the global pool doing the parallel work.
    assert_eq!(with_threads(2, rayon::current_thread_index), None);
    assert_eq!(with_threads(2, rayon::current_num_threads), 2);
    // A nested call (domain_search's domainate) runs in the enclosing pool.
    let nested: Vec<usize> = with_threads(2, || {
        (0..16)
            .into_par_iter()
            .map(|_| with_threads(0, rayon::current_num_threads))
            .collect()
    });
    assert!(nested.iter().all(|&n| n == 2), "{nested:?}");

    let fixture = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../test/data/pdonr_hmms.hmm"
    );
    let hmms = domainator_core::search::hmm_read::read_hmm_file(std::path::Path::new(fixture))
        .expect("reading the fixture");
    assert_eq!(hmms.len(), 7);

    // Two pool threads, the main thread and the test's. A dropped pool's threads
    // exit asynchronously, so one built above would very likely still be counted.
    #[cfg(target_os = "linux")]
    {
        let status = std::fs::read_to_string("/proc/self/status").unwrap();
        let threads: usize = status
            .lines()
            .find_map(|line| line.strip_prefix("Threads:"))
            .and_then(|n| n.trim().parse().ok())
            .unwrap();
        assert!(threads <= 4, "{threads} threads");
    }
}
