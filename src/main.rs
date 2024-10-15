use timeit::timeit_loops;
use pyevereader::eve_process::eve_process::EVEProcess;
use std::io;
use timeit::timeit;
use rayon::prelude::*;

#[profiling::function]
fn main() -> io::Result<()> {
    profiling::scope!("eve");
    rayon::ThreadPoolBuilder::new().num_threads(4).build_global().unwrap();
    let mut found: Vec<EVEProcess> = Vec::new();
    timeit!({
        found = EVEProcess::list().unwrap();
    });
    let mut proc = found.remove(0);
    timeit!({
        proc.init();
    });
    // let &ui_root_type = proc.search_type("UIRoot", None).get(0).unwrap();
    println!("type: {}", proc.py_type);
    println!("UIRoot type: 0x{:X}", proc.ui_root);
    for ui_root_candidate in proc.search_ui_root(None)? {
        println!("{:?}", ui_root_candidate);
    }
    profiling::finish_frame!();
    Ok(())
}