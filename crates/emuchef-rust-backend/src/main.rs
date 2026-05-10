use std::io::{self, Read};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let input = if args == ["--sidecar"] {
        let mut input = String::new();
        if let Err(error) = io::stdin().read_to_string(&mut input) {
            eprintln!("failed to read stdin: {error}");
            std::process::exit(1);
        }
        input
    } else {
        String::new()
    };

    let output = emuchef_rust_backend::run_with_args_and_input(&args, &input);
    if !output.stdout.is_empty() {
        print!("{}", output.stdout);
    }
    if !output.stderr.is_empty() {
        eprint!("{}", output.stderr);
    }
    std::process::exit(output.exit_code);
}
