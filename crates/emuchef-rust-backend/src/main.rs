use std::io;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args == ["--sidecar"] {
        let stdin = io::stdin();
        let stdout = io::stdout();
        if let Err(error) =
            emuchef_rust_backend::jsonl::run_jsonl_sidecar(stdin.lock(), stdout.lock())
        {
            eprintln!("sidecar failed: {error}");
            std::process::exit(1);
        }
        std::process::exit(0);
    }

    let output = emuchef_rust_backend::run_with_args_and_input(&args, "");
    if !output.stdout.is_empty() {
        print!("{}", output.stdout);
    }
    if !output.stderr.is_empty() {
        eprint!("{}", output.stderr);
    }
    std::process::exit(output.exit_code);
}
