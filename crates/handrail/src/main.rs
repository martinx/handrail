//! Handrail command-line interface.
//!
//! 0.0.x is an early preview. The working installer today is the POSIX-sh prototype in the
//! repository (`install.sh`); this binary grows into its replacement in milestone M1
//! (see docs/plan.md). Until then it only reports its version and where to start, and it
//! changes nothing on the machine.

use std::process::ExitCode;

const VERSION: &str = env!("CARGO_PKG_VERSION");
const REPOSITORY: &str = env!("CARGO_PKG_REPOSITORY");

fn help() -> String {
    format!(
        "handrail {VERSION}
Policy packs for coding agents: enforced where the agent allows it, honest where it doesn't.

USAGE:
    handrail [--version | --help]

This is an early preview and does not change anything on your machine yet.
To install policy for Claude Code today, use the installer in the repository:

    git clone {REPOSITORY}
    cd handrail
    sh install.sh --list
    sh install.sh --profile baseline --dry-run
    sudo sh install.sh --profile baseline

Unofficial project; not affiliated with Anthropic or any agent vendor.
Licensed under Apache-2.0. {REPOSITORY}
"
    )
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        None | Some("-h" | "--help" | "help") => {
            print!("{}", help());
            ExitCode::SUCCESS
        }
        Some("-V" | "--version" | "version") => {
            println!("handrail {VERSION}");
            ExitCode::SUCCESS
        }
        Some(other) => {
            eprintln!("handrail: unknown command '{other}'. Run 'handrail --help'.");
            ExitCode::from(2)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn help_mentions_the_version_and_the_repository() {
        let h = help();
        assert!(h.contains(VERSION));
        assert!(h.contains(REPOSITORY));
    }

    #[test]
    fn help_says_nothing_is_changed() {
        assert!(help().contains("does not change anything"));
    }
}
