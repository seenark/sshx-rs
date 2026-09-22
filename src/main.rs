use std::{env, ffi::OsString, process};

const USAGE: &str = "Usage: sshx [--version]";

fn main() {
    let mut args = env::args_os();
    let _program = args.next();

    match args.next() {
        None => println!("{USAGE}"),
        Some(value) if value == "--help" || value == "-h" => println!("{USAGE}"),
        Some(value) if value == "--version" || value == "-V" => println!("{}", sshx::VERSION),
        Some(value) => fail(value),
    }
}

fn fail(argument: OsString) -> ! {
    eprintln!(
        "sshx: unexpected argument `{}`\n{USAGE}",
        argument.to_string_lossy()
    );
    process::exit(2);
}
