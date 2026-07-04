use std::path::{Path, PathBuf};

use clap::{ArgAction, Parser, Subcommand};

#[derive(Debug, Parser)]
#[command(
    name = "jgrep",
    version,
    about = "grep for JSON, NDJSON, and YAML using jq filters",
    override_usage = "jgrep [OPTIONS] FILTER [FILE...]\njgrep explore [OPTIONS] [FILE]\njgrep completion SHELL"
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

    #[arg(
        short = 'p',
        long = "path",
        value_name = "FIELD",
        help = "Extract a dotted field path without jq syntax, e.g. -p app.level"
    )]
    pub path_filter: Option<String>,

    #[arg(
        short = 'w',
        long = "where",
        value_name = "TEST",
        action = ArgAction::Append,
        help = "Filter with a simple condition, e.g. -w status=active or -w age>=18"
    )]
    pub where_filters: Vec<String>,

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
    #[command(about = "Explore JSON, NDJSON, or YAML with schema hints and a live preview")]
    Explore(ExploreArgs),

    #[command(about = "Generate shell completion script")]
    Completion {
        #[arg(
            value_name = "SHELL",
            help = "Shell to generate completion for: bash, zsh, fish, powershell"
        )]
        shell: String,
    },
}

#[derive(Debug, Parser)]
pub struct ExploreArgs {
    #[arg(
        index = 1,
        value_name = "FILE",
        help = "File to explore; reads stdin if omitted"
    )]
    pub input: Option<PathBuf>,

    #[arg(
        short = 'f',
        long = "filter",
        value_name = "EXPR",
        help = "Initial jq filter or shortcut expression"
    )]
    pub filter: Option<String>,

    #[arg(
        long = "pretty",
        help = "Pretty-print JSON output printed from explore"
    )]
    pub pretty: bool,

    #[arg(
        short = 'p',
        long = "path",
        value_name = "EXPR",
        help = "Set the explore Output expression, e.g. -p message or -p .message"
    )]
    pub path_filter: Option<String>,

    #[arg(
        short = 'w',
        long = "where",
        value_name = "TEST",
        action = ArgAction::Append,
        help = "Filter with a simple condition, e.g. -w log.level=ERROR"
    )]
    pub where_filters: Vec<String>,

    #[arg(
        long = "no-color",
        help = "Disable colored output printed from explore"
    )]
    pub no_color: bool,

    #[arg(
        short = 'C',
        long = "color-level",
        help = "Color printed output by log level"
    )]
    pub color_level: bool,

    #[arg(
        long = "color-level-field",
        value_name = "FIELD",
        help = "Field used by --color-level"
    )]
    pub color_level_field: Option<String>,

    #[arg(
        long = "print",
        help = "Print a deterministic schema and preview snapshot instead of opening the TUI"
    )]
    pub print: bool,

    #[arg(
        long = "max-input-bytes",
        default_value_t = 50 * 1024 * 1024,
        value_name = "BYTES",
        help = "Refuse to load more than this many input bytes"
    )]
    pub max_input_bytes: usize,

    #[arg(
        long = "max-schema-documents",
        default_value_t = 500,
        value_name = "N",
        help = "Maximum documents sampled for schema inference"
    )]
    pub max_schema_documents: usize,

    #[arg(
        long = "max-preview-results",
        default_value_t = 30,
        value_name = "N",
        help = "Maximum matching results shown in the live preview"
    )]
    pub max_preview_results: usize,

    #[arg(
        long = "max-schema-depth",
        default_value_t = 8,
        value_name = "N",
        help = "Maximum nested depth inspected during schema inference"
    )]
    pub max_schema_depth: usize,
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
}
