#![forbid(unsafe_code)]
use rhost::output;
use std::io::{self, Write};
use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let json = args
        .iter()
        .take_while(|a| a.as_str() != "--")
        .any(|a| a == "--json" || a == "--json=true");
    let operands: Vec<&str> = args
        .iter()
        .filter(|a| a.as_str() != "--json" && a.as_str() != "--json=true")
        .map(String::as_str)
        .collect();
    let mut stdout = io::stdout().lock();
    let (status, delivered) = match operands.as_slice() {
        ["version"] | ["--version"] => (
            0,
            if json {
                output::version().write(&mut stdout)
            } else {
                writeln!(
                    stdout,
                    "rhost {} (Rust migration skeleton)",
                    env!("CARGO_PKG_VERSION")
                )
            },
        ),
        [] | ["--help"] | ["-h"] if !json => (
            0,
            writeln!(
                stdout,
                "rhost: Rust migration skeleton\nUsage: rhost version [--json]\nRemote operations are not implemented yet."
            ),
        ),
        _ => {
            let message = "Rust migration skeleton supports only version; use the Go binary for remote operations";
            if json {
                (255, output::usage(message).write(&mut stdout))
            } else {
                eprintln!("rhost: USAGE_ERROR: {message}");
                (255, Ok(()))
            }
        }
    };
    if let Err(error) = delivered {
        eprintln!("rhost: OUTPUT_WRITE_FAILED: {error}");
        return ExitCode::from(255);
    }
    ExitCode::from(status)
}
