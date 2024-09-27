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
    println!("0x{:X}", proc.search_type("UIRoot", None).get(0).unwrap());
    Ok(())
}