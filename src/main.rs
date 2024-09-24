use pyevereader::eve_process::process::*;
use tracing;
use tracing::Level;
use tracing_subscriber;
use tracing_subscriber::filter;

fn main() {
    tracing_subscriber::fmt::init();
    tracing::subscriber::set_global_default(
        tracing_subscriber::fmt().with_level(true).with_max_level(Level::INFO).finish()
    );

    let procs = Process::list(None, Some("*exefile*"), Some("*星战前夜*"));
    println!("{:?}", procs);
    for mut proc in procs.unwrap() {
        println!("{:?}", proc);
        proc.enum_memory_regions();
        println!("{:?}", proc.regions);
    }
}