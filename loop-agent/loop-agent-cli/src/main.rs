use std::{env, ffi::OsStr, process};

fn main() {
    let mut args = env::args_os().skip(1);
    let first_arg = args.next();
    let is_version_flag = first_arg
        .as_deref()
        .is_some_and(|arg| arg == OsStr::new("--version") || arg == OsStr::new("-V"));

    if is_version_flag {
        println!("loop {}", env!("CARGO_PKG_VERSION"));
        return;
    }

    eprintln!("{}", loop_agent_core::m0_runtime_notice());
    process::exit(64);
}
