//! Sequence search: hmmer-pure-rs wrappers and the [`SearchResult`] model that
//! domainate/domain_search turn into `Domainator` GenBank features.

pub mod hmm_compare;
pub mod hmmer;
pub mod nhmmer;
pub mod references;
pub mod results;
pub mod seq_compare;

pub use results::SearchResult;
