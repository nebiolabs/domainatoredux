use domainator_core::io::shards::resolve_database_shards;
use std::path::Path;
fn main() {
    let a: Vec<String> = std::env::args().collect();
    match resolve_database_shards(Path::new(&a[1])) {
        Ok(v) => {
            for p in v {
                println!("{}", p.display());
            }
        }
        Err(e) => {
            eprintln!("ERR: {}", e);
            std::process::exit(3);
        }
    }
}
