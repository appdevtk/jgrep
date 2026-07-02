use std::path::{Path, PathBuf};

use clap::{ArgAction, Parser, Subcommand};

#[derive(Debug, Parser)]
#[command(
    name = "jgrep",
    version,
    about = "grep for JSON, NDJSON, and YAML using jq filters",
    override_usage = "jgrep [OPTIONS] FILTER [FILE...]\njgrep completion SHELL"
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Option<Command>,

    #[arg(short = 'r', long = "recursive", help = "Recurse into directories")]
    pub recursive: bool,

    #[arg(
        short = 'l',
        long = "files-with-matches",
        help = "Only print filenames with matches"
    )]
    pub files_with_matches: bool,

    #[arg(short = 'c', long = "count", help = "Print match count per file")]
    pub count: bool,

    #[arg(
        short = 's',
        long = "slurp",
        help = "Collect all results into a single JSON array"
    )]
    pub slurp: bool,

    #[arg(
        short = 'n',
        long = "null-input",
        help = "Use null as input instead of reading files"
    )]
    pub null_input: bool,

    #[arg(
        short = 'f',
        long = "from-file",
        value_name = "FILE",
        help = "Read filter expression from a file"
    )]
    pub from_file: Option<PathBuf>,

    #[arg(long = "pretty", help = "Pretty-print JSON output")]
    pub pretty: bool,

    #[arg(long = "no-color", help = "Disable colored output")]
    pub no_color: bool,

    #[arg(
        short = 'C',
        long = "color-level",
        help = "Color each output line by log level"
    )]
    pub color_level: bool,

    #[arg(
        long = "color-level-field",
        value_name = "FIELD",
        help = "Field used by --color-level"
    )]
    pub color_level_field: Option<String>,

    #[arg(
        index = 1,
        help = "jq filter; defaults to '.' when omitted or when this argument is an existing path"
    )]
    pub filter: Option<String>,

    #[arg(index = 2, action = ArgAction::Append, help = "Files to search (reads from stdin if omitted)")]
    pub files: Vec<PathBuf>,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    #[command(about = "Generate shell completion script")]
    Completion {
        #[arg(
            value_name = "SHELL",
            help = "Shell to generate completion for: bash, zsh, fish, powershell"
        )]
        shell: String,
    },
}

pub fn parse() -> Result<Cli, i32> {
    Ok(Cli::parse())
}

impl Cli {
    pub fn filter_expression(&self) -> Option<String> {
        if self.from_file.is_some() {
            return None;
        }

        match self.filter.as_deref() {
            Some(candidate) if !Path::new(candidate).exists() => Some(candidate.to_owned()),
            _ => Some(".".to_owned()),
        }
    }

    pub fn input_paths(&self) -> Vec<PathBuf> {
        let mut paths = Vec::new();
        if self.from_file.is_some() || self.filter_is_existing_path() {
            if let Some(filter_position) = &self.filter {
                paths.push(PathBuf::from(filter_position));
            }
        }
        paths.extend(self.files.iter().cloned());
        paths
    }

    fn filter_is_existing_path(&self) -> bool {
        self.filter
            .as_deref()
            .is_some_and(|candidate| Path::new(candidate).exists())
    }

    pub fn use_json_color(&self) -> bool {
        if self.no_color || std::env::var_os("NO_COLOR").is_some() {
            return false;
        }
        std::io::IsTerminal::is_terminal(&std::io::stdout())
    }

    pub fn use_level_color(&self) -> bool {
        self.color_level && !self.no_color && std::env::var_os("NO_COLOR").is_none()
    }
}
