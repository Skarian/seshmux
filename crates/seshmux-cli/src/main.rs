fn main() {
    if let Err(error) = seshmux_cli::run() {
        eprintln!("Error: {error}");
        std::process::exit(1);
    }
}
