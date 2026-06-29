//! Sequence search: hmmer-pure-rs wrappers and the [`SearchResult`] model that
//! domainate/domain_search turn into `Domainator` GenBank features.

pub mod hmmer;
pub mod results;

pub use results::SearchResult;
