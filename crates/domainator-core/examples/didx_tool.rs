use std::path::Path;
use domainator_core::io::{didx, FileType, Compression};
use domainator_core::io::genbank::offsets::genbank_offsets;
fn main(){
    let a:Vec<String>=std::env::args().collect();
    match a[1].as_str() {
        "write" => { // didx_tool write <src.gb>  -> writes <src.gb>.didx
            let pairs=genbank_offsets(&a[2]).unwrap();
            let offs:Vec<u64>=pairs.iter().map(|p|p.0).collect();
            let cds:Vec<u64>=pairs.iter().map(|p|p.1).collect();
            let p=didx::write_index(Path::new(&a[2]),&offs,&cds,FileType::Genbank,Compression::None).unwrap();
            println!("wrote {}", p.display());
        }
        "read" => { // didx_tool read <src.gb>
            match didx::read_index(Path::new(&a[2]),FileType::Genbank,Compression::None).unwrap() {
                Some((o,c)) => for (x,y) in o.iter().zip(&c){ println!("{} {}",x,y); },
                None => { eprintln!("STALE/MISS"); std::process::exit(2);}
            }
        }
        _=>{}
    }
}
