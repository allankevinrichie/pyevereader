use std::time::Instant;
use pyevereader::eve_process::process::*;
use tracing;
use tracing::Level;
use tracing_subscriber;
use tracing_subscriber::filter;
use rayon::prelude::*;

fn main() {
    rayon::ThreadPoolBuilder::new().num_threads(4).build_global().unwrap();

    let procs = Process::list(None, Some("*exefile*"), Some("*星战前夜*"));
    println!("{:?}", procs);
    for mut proc in procs.unwrap() {
        println!("{:?}", proc);
        proc.enum_memory_regions();
        // println!("{:?}", proc.regions);
        let now = Instant::now();
        proc.sync_memory_regions();
        println!("{:?}", now.elapsed());
        // println!("{:?}", proc.regions)
    }
}