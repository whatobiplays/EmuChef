fn main() {
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    let output = emuchef_rust_backend::plan_shadow::run(&args);
    if !output.stdout.is_empty() {
        print!("{}", output.stdout);
    }
    if !output.stderr.is_empty() {
        eprint!("{}", output.stderr);
    }
    std::process::exit(output.exit_code);
}
