//! Sequence search: hmmer-pure-rs and infernox wrappers and the [`SearchResult`] model that
//! domainate/domain_search turn into `Domainator` GenBank features.

pub mod foldseek;
pub mod hmm_compare;
pub mod hmm_read;
pub mod hmm_write;
pub mod hmmer;
pub mod infernal;
pub mod nhmmer;
pub mod references;
pub mod results;
pub mod seq_compare;

pub use results::SearchResult;
