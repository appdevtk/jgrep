mod cli;
mod color;
mod completion;
mod discovery;
mod input;
mod matcher;
mod output;
mod shortcuts;

use std::io::{self, Read, Write};

use cli::{Cli, Command};
use input::{InputKind, Source};
use matcher::Matcher;

pub use color::color_for_level;

const NAME: &str = "jgrep";

pub fn main_entry() -> i32 {
    let cli = match cli::parse() {
        Ok(cli) => cli,
        Err(code) => return code,
    };

    match cli.command {
        Some(Command::Completion { shell }) => completion::print_completion(&shell),
        None => run(cli),
    }
}

fn run(cli: Cli) -> i32 {
    let mut out = io::BufWriter::new(io::stdout());
    let mut err = io::BufWriter::new(io::stderr());

    let filter = match resolve_filter(&cli, &mut err) {
        Ok(filter) => filter,
        Err(()) => return 2,
    };

    if cli.count && cli.files_with_matches {
        let _ = writeln!(
            err,
            "{NAME}: --count and --files-with-matches cannot be used together"
        );
        return 2;
    }

    let matcher = match Matcher::compile(&filter) {
        Ok(matcher) => matcher,
        Err(e) => {
            let _ = writeln!(err, "{NAME}: invalid filter: {e}");
            return 2;
        }
    };

    let mut state = RunState::default();
    let mut slurp = Vec::new();

    if cli.null_input {
        process_value(
            &cli,
            &matcher,
            None,
            false,
            jaq_json::Val::Null,
            &mut out,
            &mut err,
            &mut state,
            &mut slurp,
        );
    } else {
        match discovery::resolve(&cli.input_paths(), cli.recursive) {
            Ok(None) => process_stdin(&cli, &matcher, &mut out, &mut err, &mut state, &mut slurp),
            Ok(Some(files)) => {
                let show_filename = files.len() > 1;
                for source in files {
                    process_file(
                        &cli,
                        &matcher,
                        source,
                        show_filename,
                        &mut out,
                        &mut err,
                        &mut state,
                        &mut slurp,
                    );
                }
            }
            Err(errors) => {
                for error in errors {
                    let _ = writeln!(err, "{NAME}: {error}");
                }
                state.had_error = true;
            }
        }
    }

    if cli.slurp {
        state.found_match = !slurp.is_empty();
        if let Err(e) = output::write_slurp(&mut out, &slurp, cli.pretty, cli.use_json_color()) {
            let _ = writeln!(err, "{NAME}: error formatting output: {e}");
            state.had_error = true;
        }
    }

    let _ = out.flush();
    let _ = err.flush();

    if state.had_error {
        2
    } else if state.found_match {
        0
    } else {
        1
    }
}

fn resolve_filter(cli: &Cli, err: &mut dyn Write) -> Result<String, ()> {
    let base_filter = if let Some(path_filter) = &cli.path_filter {
        shortcuts::path_filter(path_filter).map_err(|e| {
            let _ = writeln!(err, "{NAME}: invalid path shortcut: {e}");
        })?
    } else if let Some(path) = &cli.from_file {
        std::fs::read_to_string(path)
            .map(|s| s.trim().to_owned())
            .map_err(|e| {
                let _ = writeln!(err, "{NAME}: cannot read filter file: {e}");
            })?
    } else {
        cli.filter_expression().ok_or_else(|| {
            let _ = writeln!(
                err,
                "{NAME}: filter expression required (or use -f to read from file)"
            );
        })?
    };

    shortcuts::apply_where_filters(base_filter, &cli.where_filters).map_err(|e| {
        let _ = writeln!(err, "{NAME}: invalid where shortcut: {e}");
    })
}

fn process_stdin(
    cli: &Cli,
    matcher: &Matcher,
    out: &mut dyn Write,
    err: &mut dyn Write,
    state: &mut RunState,
    slurp: &mut Vec<jaq_json::Val>,
) {
    let mut bytes = Vec::new();
    if let Err(e) = io::stdin().read_to_end(&mut bytes) {
        let _ = writeln!(err, "{NAME}: stdin: {e}");
        state.had_error = true;
        return;
    }
    process_bytes(
        cli,
        matcher,
        Source::Stdin,
        false,
        input::detect_stdin_kind(&bytes),
        bytes,
        out,
        err,
        state,
        slurp,
    );
}

#[allow(clippy::too_many_arguments)]
fn process_file(
    cli: &Cli,
    matcher: &Matcher,
    source: Source,
    show_filename: bool,
    out: &mut dyn Write,
    err: &mut dyn Write,
    state: &mut RunState,
    slurp: &mut Vec<jaq_json::Val>,
) {
    let bytes = match std::fs::read(source.path()) {
        Ok(bytes) => bytes,
        Err(e) => {
            let _ = writeln!(err, "{NAME}: {}: {e}", source.label());
            state.had_error = true;
            return;
        }
    };
    let kind = input::kind_for_path(source.path()).unwrap_or(InputKind::Json);
    process_bytes(
        cli,
        matcher,
        source,
        show_filename,
        kind,
        bytes,
        out,
        err,
        state,
        slurp,
    );
}

#[allow(clippy::too_many_arguments)]
fn process_bytes(
    cli: &Cli,
    matcher: &Matcher,
    source: Source,
    show_filename: bool,
    kind: InputKind,
    bytes: Vec<u8>,
    out: &mut dyn Write,
    err: &mut dyn Write,
    state: &mut RunState,
    slurp: &mut Vec<jaq_json::Val>,
) {
    let values = match input::parse_many(kind, &bytes) {
        Ok(values) => values,
        Err(e) if matches!(source, Source::Stdin) && kind == InputKind::Json => {
            match input::parse_many(InputKind::Yaml, &bytes) {
                Ok(values) => values,
                Err(_) => {
                    let _ = writeln!(err, "{NAME}: {}: parse error: {e}", source.label());
                    state.had_error = true;
                    return;
                }
            }
        }
        Err(e) => {
            let _ = writeln!(err, "{NAME}: {}: parse error: {e}", source.label());
            state.had_error = true;
            return;
        }
    };

    let values = if matches!(source, Source::Stdin)
        && kind == InputKind::Json
        && values.first().is_some_and(Result::is_err)
    {
        match input::parse_many(InputKind::Yaml, &bytes) {
            Ok(values) => values,
            Err(_) => values,
        }
    } else {
        values
    };

    let mut match_count = 0usize;
    for value in values {
        let value = match value {
            Ok(value) => value,
            Err(e) => {
                let _ = writeln!(err, "{NAME}: {}: parse error: {e}", source.label());
                state.had_error = true;
                continue;
            }
        };
        match_count += process_value(
            cli,
            matcher,
            Some(source.label()),
            show_filename,
            value,
            out,
            err,
            state,
            slurp,
        );
    }

    if !cli.slurp {
        if cli.count {
            if show_filename {
                let _ = write!(out, "{}:", source.label());
            }
            let _ = writeln!(out, "{match_count}");
        } else if cli.files_with_matches && match_count > 0 {
            let _ = writeln!(out, "{}", source.label());
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn process_value(
    cli: &Cli,
    matcher: &Matcher,
    filename: Option<String>,
    show_filename: bool,
    value: jaq_json::Val,
    out: &mut dyn Write,
    err: &mut dyn Write,
    state: &mut RunState,
    slurp: &mut Vec<jaq_json::Val>,
) -> usize {
    let results = match matcher.apply(value.clone()) {
        Ok(results) => results,
        Err(e) => {
            let _ = writeln!(err, "{NAME}: filter error: {e}");
            state.had_error = true;
            return 0;
        }
    };

    let mut count = 0usize;
    for result in results {
        if !matcher::is_match(&result) {
            continue;
        }
        count += 1;
        state.found_match = true;
        if cli.slurp {
            slurp.push(result);
        } else if !cli.count
            && !cli.files_with_matches
            && output::write_result(
                out,
                &result,
                filename.as_deref(),
                show_filename,
                &value,
                cli,
            )
            .is_err()
        {
            state.had_error = true;
        }
    }
    count
}

#[derive(Default)]
struct RunState {
    found_match: bool,
    had_error: bool,
}
