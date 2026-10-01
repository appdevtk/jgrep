use std::path::{Path, PathBuf};

use clap::{ArgAction, CommandFactory, Parser, Subcommand};

#[derive(Debug, Parser)]
#[command(
    name = "jgrep",
    version,
    about = "grep for JSON, NDJSON, and YAML using jq filters",
    override_usage = "jgrep [OPTIONS] [FILTER] [FILE...]\njgrep --filter EXPR [OPTIONS] [FILE...]\njgrep explore [OPTIONS] [FILE]\njgrep completion SHELL",
    after_help = "Examples:\n  jgrep users.json                         Show all records\n  jgrep -p name users.ndjson               Extract a field\n  jgrep -w 'age>=18' -p name users.ndjson   Filter and extract\n  jgrep --filter '.items[]' data.json      Use an explicit jq filter\n  jgrep -r -c -w status=active data/       Count matches recursively\n  cat logs.ndjson | jgrep -p message       Read piped data\n  jgrep explore users.json                 Explore fields interactively\n\nExit codes: 0 = matches, 1 = no matches, 2 = error.\nQuote filters and conditions containing shell operators such as > or |."
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

    #[arg(
        long = "filter",
        value_name = "EXPR",
        conflicts_with_all = ["from_file", "path_filter"],
        help = "Use an explicit jq filter; positional arguments are input paths"
    )]
    pub filter_override: Option<String>,

    #[arg(long = "pretty", help = "Pretty-print JSON output")]
    pub pretty: bool,

    #[arg(
        short = 'p',
        long = "path",
        conflicts_with = "from_file",
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
    #[command(about = "Read-only k9s bridge and plugin installer (requires explore support)")]
    K9s(K9sArgs),
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
pub struct K9sArgs {
    #[command(subcommand)]
    pub action: K9sAction,
}

#[derive(Debug, Subcommand)]
pub enum K9sAction {
    /// Follow normalized JSON/text logs from the selected pod or container.
    Logs(K9sSelection),
    /// Explore an allowlisted resource as JSON without changing it.
    Resource(K9sSelection),
    /// Install a private binary snapshot without overwriting existing plugins.
    Install {
        #[arg(long)]
        config_dir: PathBuf,
        #[arg(long)]
        jgrep_bin: Option<PathBuf>,
        #[arg(long, default_value = "kubectl")]
        kubectl: PathBuf,
        /// Back up an existing snapshot outside scanned plugin directories.
        #[arg(long)]
        update: bool,
    },
}

#[derive(Debug, Parser)]
pub struct K9sSelection {
    #[arg(long)]
    pub context: String,
    #[arg(long)]
    pub namespace: String,
    #[arg(long)]
    pub name: String,
    #[arg(long, default_value = "")]
    pub container: String,
    #[arg(long, default_value = "pods")]
    pub resource: String,
    #[arg(long, default_value = "")]
    pub group: String,
    #[arg(long, default_value = "")]
    pub kubeconfig: String,
    #[arg(long, default_value = "kubectl")]
    pub kubectl: PathBuf,
    /// Override the viewer executable, mainly for controlled integration tests.
    #[arg(long, hide = true)]
    pub jgrep: Option<PathBuf>,
    #[arg(long)]
    pub state_dir: Option<PathBuf>,
    #[arg(long, default_value_t = 10 * 1024 * 1024, value_parser = clap::value_parser!(u64).range(1..))]
    pub max_input_bytes: u64,
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
        long,
        conflicts_with_all = ["input", "print"],
        help = "Open the TUI immediately for NDJSON stdin, even before the first log line"
    )]
    pub stream_json: bool,

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
    if std::env::args_os().len() == 1 && std::io::IsTerminal::is_terminal(&std::io::stdin()) {
        return Err(if Cli::command().print_help().is_ok() {
            0
        } else {
            2
        });
    }
    Ok(Cli::parse())
}

impl Cli {
    pub fn filter_expression(&self) -> Option<String> {
        if self.from_file.is_some() {
            return None;
        }
        if let Some(filter) = &self.filter_override {
            return Some(filter.clone());
        }

        match self.filter.as_deref() {
            Some(candidate) if !self.first_position_is_input() => Some(candidate.to_owned()),
            _ => Some(".".to_owned()),
        }
    }

    pub fn input_paths(&self) -> Vec<PathBuf> {
        let mut paths = Vec::new();
        if self.first_position_is_input() {
            if let Some(filter_position) = &self.filter {
                paths.push(PathBuf::from(filter_position));
            }
        }
        paths.extend(self.files.iter().cloned());
        paths
    }

    fn first_position_is_input(&self) -> bool {
        let Some(candidate) = self.filter.as_deref() else {
            return false;
        };
        if self.from_file.is_some() || self.path_filter.is_some() || self.filter_override.is_some()
        {
            return true;
        }
        // `.` and `..` are jq expressions as well as real directories.
        // Preserve the convenient `jgrep -r .` directory-only invocation.
        if matches!(candidate, "." | "..") {
            return self.recursive && self.files.is_empty();
        }
        let path = Path::new(candidate);
        if path.exists() {
            return true;
        }
        let looks_like_path = path.is_absolute()
            || candidate.starts_with("./")
            || candidate.starts_with("../")
            || (!candidate.starts_with('.')
                && !candidate.contains(|ch: char| ch.is_whitespace() || "|()[]{}\"'".contains(ch))
                && crate::input::kind_for_path(path).is_some());
        // Expressions such as `./length` or `length/.json` look like paths.
        // Prefer a valid jq expression over guessing a nonexistent filename.
        looks_like_path && crate::matcher::Matcher::compile(candidate).is_err()
    }

    pub fn use_json_color(&self) -> bool {
        if self.no_color || std::env::var_os("NO_COLOR").is_some() {
            return false;
        }
        std::io::IsTerminal::is_terminal(&std::io::stdout())
    }
}
