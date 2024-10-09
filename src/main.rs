use std::io;
use std::time::Instant;
use pyevereader::eve_process::process::*;
use tracing;
use tracing::Level;
use tracing_subscriber;
use tracing_subscriber::filter;
use rayon::prelude::*;
use pyevereader::eve_process::eve_process::EVEProcess;

fn main() -> io::Result<()> {
    rayon::ThreadPoolBuilder::new().num_threads(4).build_global().unwrap();
    let mut found = EVEProcess::list().unwrap();
    let mut proc = found.remove(0);
    proc.init();
    // let &ui_root_type = proc.search_type("UIRoot", None).get(0).unwrap();
    println!("type: {}", proc.py_type.base_addr);
    println!("UIRoot type: 0x{:X}", proc.ui_root.base_addr);
    for ui_root_candidate in proc.search_ui_root(None) {
        println!("{:?}", ui_root_candidate);
    }
    Ok(())
}