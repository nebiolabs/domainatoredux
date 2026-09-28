use domainator_core::io::fasta::{read_fasta, write_fasta};
fn main() {
    let a: Vec<String> = std::env::args().collect();
    let r = read_fasta(&a[1], Some("protein")).unwrap();
    write_fasta(&a[2], &r).unwrap();
}
