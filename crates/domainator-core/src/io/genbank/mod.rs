//! GenBank reading, writing, and offset scanning.

pub mod offsets;
pub mod parse;
pub mod reader;
pub mod writer;

pub use reader::{read_genbank, scan_genbank_ids, GenbankReader};
pub use writer::write_genbank;
