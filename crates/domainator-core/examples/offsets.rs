use domainator_core::io::genbank::offsets::{genbank_offsets, fasta_offsets};
fn main() {
    let mut a = std::env::args().skip(1);
    let kind = a.next().unwrap();
    let path = a.next().unwrap();
    let pairs = if kind == "gb" { genbank_offsets(&path).unwrap() } else { fasta_offsets(&path).unwrap() };
    for (o, c) in pairs { println!("{} {}", o, c); }
}
