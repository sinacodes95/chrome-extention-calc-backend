use rust_calculator::{calculate_json, MAX_REQUEST_BYTES};
use std::io::{self, Read, Write};

fn main() -> io::Result<()> {
    if let Some(argument) = std::env::args_os().nth(1) {
        if argument == "-h" || argument == "--help" {
            println!("Usage: rust-calculator < request.json\nRead one UTF-8 JSON request and write one JSON response.");
            return Ok(());
        }
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "use --help or pass a JSON request on standard input",
        ));
    }

    let mut input = Vec::new();
    io::stdin()
        .lock()
        .take((MAX_REQUEST_BYTES + 1) as u64)
        .read_to_end(&mut input)?;
    let response = calculate_json(input);
    writeln!(io::stdout().lock(), "{response}")
}
