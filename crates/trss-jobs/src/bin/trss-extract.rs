//! The child process that unpacks a received archive under hard limits (`trss_archive::child`).

fn main() -> std::process::ExitCode {
    trss_archive::child::main()
}
