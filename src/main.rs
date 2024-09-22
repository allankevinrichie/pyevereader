use pyevereader::eve_process::process::*;

fn main() {
    let names = list_processes().unwrap();
    println!("{:?}", names);
}