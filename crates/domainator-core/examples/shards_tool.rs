use std::path::Path;
use domainator_core::io::shards::resolve_database_shards;
fn main(){let a:Vec<String>=std::env::args().collect();
 match resolve_database_shards(Path::new(&a[1])){
   Ok(v)=>for p in v{println!("{}",p.display());},
   Err(e)=>{eprintln!("ERR: {}",e);std::process::exit(3);}
 }}
