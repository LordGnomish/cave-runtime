//! `cave-wasm` — thin CLI wrapper around [`cave_wasm_runtime::cli::dispatch`].

use std::process::exit;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match cave_wasm_runtime::cli::dispatch(&args) {
        Ok(out) => println!("{out}"),
        Err(e) => {
            eprintln!("error: {e}");
            exit(1);
        }
    }
}
