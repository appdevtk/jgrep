use clap::CommandFactory;
use clap_complete::{generate, Shell};

use crate::cli::Cli;

pub fn print_completion(shell: &str) -> i32 {
    let shell = match shell.to_ascii_lowercase().as_str() {
        "bash" => Shell::Bash,
        "zsh" => Shell::Zsh,
        "fish" => Shell::Fish,
        "powershell" | "pwsh" => Shell::PowerShell,
        other => {
            eprintln!("jgrep: unsupported shell: {other}");
            eprintln!("Supported shells: bash, zsh, fish, powershell");
            return 2;
        }
    };

    let mut cmd = Cli::command();
    generate(shell, &mut cmd, "jgrep", &mut std::io::stdout());
    0
}
