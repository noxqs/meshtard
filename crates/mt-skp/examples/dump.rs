//! Prints a .skp file as JSON in the same shape as `tools/oracle_dump.py`.
//!
//!     cargo run -p mt-skp --example dump FILE.skp

fn main() {
    let path = std::env::args().nth(1).expect("usage: dump FILE.skp");
    match mt_skp::read(&path) {
        Ok(model) => println!(
            "{}",
            serde_json::to_string_pretty(&mt_skp::dump::to_json(&model)).unwrap()
        ),
        Err(e) => {
            eprintln!("{path}: {e}");
            std::process::exit(1);
        }
    }
}
