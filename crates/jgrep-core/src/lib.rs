mod cli;
mod color;
#[cfg(feature = "completion")]
mod completion;
mod discovery;
#[cfg(feature = "explore")]
mod explore;
mod input;
#[cfg(feature = "explore")]
mod k9s;
mod matcher;
mod output;
#[cfg(feature = "explore")]
mod schema;
mod shortcuts;

use std::io::{self, BufRead, Read, Write};

use cli::Cli;
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
        Some(cli::Command::K9s(args)) => {
            #[cfg(feature = "explore")]
            {
                k9s::run(args)
            }
            #[cfg(not(feature = "explore"))]
            {
                let _ = args;
                eprintln!("{NAME}: k9s requires explore support in this build");
                2
            }
        }
        Some(cli::Command::Completion { shell }) => {
            #[cfg(feature = "completion")]
            {
                completion::print_completion(&shell)
            }
            #[cfg(not(feature = "completion"))]
            {
                let _ = shell;
                eprintln!("{NAME}: completion support is not enabled in this build");
                2
            }
        }
        Some(cli::Command::Explore(args)) => {
            #[cfg(feature = "explore")]
            {
                explore::run(args)
            }
            #[cfg(not(feature = "explore"))]
            {
                let _ = args;
                eprintln!("{NAME}: explore support is not enabled in this build");
                2
            }
        }
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

    let mut context = ProcessingContext {
        cli: &cli,
        matcher: &matcher,
        out: &mut out,
        err: &mut err,
        state: RunState::default(),
        slurp: Vec::new(),
    };

    if cli.null_input {
        context.process_value(None, false, jaq_json::Val::Null);
    } else {
        match discovery::resolve(&cli.input_paths(), cli.recursive) {
            Ok(None) => context.process_stdin(),
            Ok(Some(files)) => {
                let show_filename = files.len() > 1;
                for source in files {
                    context.process_file(source, show_filename);
                }
            }
            Err(errors) => {
                for error in errors {
                    let _ = writeln!(context.err, "{NAME}: {error}");
                }
                context.state.had_error = true;
            }
        }
    }

    if cli.slurp {
        context.state.found_match = !context.slurp.is_empty();
        if let Err(e) = output::write_slurp(
            context.out,
            &context.slurp,
            cli.pretty,
            cli.use_json_color(),
        ) {
            let _ = writeln!(context.err, "{NAME}: error formatting output: {e}");
            context.state.had_error = true;
        }
    }

    let _ = flush_output(context.out, context.err, &mut context.state);
    let _ = context.err.flush();

    if context.state.had_error {
        2
    } else if context.state.found_match {
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

struct ProcessingContext<'a> {
    cli: &'a Cli,
    matcher: &'a Matcher,
    out: &'a mut dyn Write,
    err: &'a mut dyn Write,
    state: RunState,
    slurp: Vec<jaq_json::Val>,
}

impl ProcessingContext<'_> {
    fn process_stdin(&mut self) {
        let stdin = io::stdin();
        if io::IsTerminal::is_terminal(&stdin) {
            let _ = writeln!(
                self.err,
                "{NAME}: no input provided. Pass a file path or pipe JSON, NDJSON, or YAML to stdin."
            );
            let _ = writeln!(
                self.err,
                "Use --null-input (-n) to evaluate a filter without data, or --help for examples."
            );
            self.state.had_error = true;
            return;
        }
        let mut reader = io::BufReader::new(stdin.lock());
        let mut first_line = Vec::new();
        match reader.read_until(b'\n', &mut first_line) {
            Ok(0) => {
                self.process_bytes(Source::Stdin, false, InputKind::Json, Vec::new());
                return;
            }
            Ok(_) => {}
            Err(e) => {
                let _ = writeln!(self.err, "{NAME}: stdin: {e}");
                self.state.had_error = true;
                return;
            }
        }

        if input::is_streamable_json_line(&first_line) {
            self.process_json_lines_stdin(first_line, reader);
            return;
        }

        let mut bytes = Vec::new();
        bytes.extend_from_slice(&first_line);
        if let Err(e) = reader.read_to_end(&mut bytes) {
            let _ = writeln!(self.err, "{NAME}: stdin: {e}");
            self.state.had_error = true;
            return;
        }
        self.process_bytes(
            Source::Stdin,
            false,
            input::detect_stdin_kind(&bytes),
            bytes,
        );
    }

    fn process_json_lines_stdin(
        &mut self,
        first_line: Vec<u8>,
        mut reader: io::BufReader<io::StdinLock<'_>>,
    ) {
        let mut match_count = 0usize;
        let mut line = first_line;

        loop {
            if !input::trim_ascii_whitespace(&line).is_empty() {
                match_count += self.process_json_line(&line);
                if flush_output(self.out, self.err, &mut self.state).is_err() {
                    return;
                }
            }

            line.clear();
            match reader.read_until(b'\n', &mut line) {
                Ok(0) => break,
                Ok(_) => {}
                Err(e) => {
                    let _ = writeln!(self.err, "{NAME}: stdin: {e}");
                    self.state.had_error = true;
                    break;
                }
            }
        }

        if !self.cli.slurp {
            if self.cli.count {
                let _ = writeln!(self.out, "{match_count}");
            } else if self.cli.files_with_matches && match_count > 0 {
                let _ = writeln!(self.out, "stdin");
            }
        }
    }

    fn process_json_line(&mut self, line: &[u8]) -> usize {
        let values = match input::parse_many(InputKind::Json, line) {
            Ok(values) => values,
            Err(e) => {
                let _ = writeln!(self.err, "{NAME}: stdin: parse error: {e}");
                self.state.had_error = true;
                return 0;
            }
        };

        let mut match_count = 0usize;
        for value in values {
            match value {
                Ok(value) => {
                    match_count += self.process_value(Some("stdin".to_owned()), false, value);
                }
                Err(e) => {
                    let _ = writeln!(self.err, "{NAME}: stdin: parse error: {e}");
                    self.state.had_error = true;
                }
            }
        }
        match_count
    }

    fn process_file(&mut self, source: Source, show_filename: bool) {
        let bytes = match std::fs::read(source.path()) {
            Ok(bytes) => bytes,
            Err(e) => {
                let _ = writeln!(self.err, "{NAME}: {}: {e}", source.label());
                self.state.had_error = true;
                return;
            }
        };
        let kind = input::kind_for_path(source.path()).unwrap_or(InputKind::Json);
        self.process_bytes(source, show_filename, kind, bytes);
    }

    fn process_bytes(
        &mut self,
        source: Source,
        show_filename: bool,
        kind: InputKind,
        bytes: Vec<u8>,
    ) {
        let values = match input::parse_many(kind, &bytes) {
            Ok(values) => values,
            Err(e) => {
                let _ = writeln!(self.err, "{NAME}: {}: parse error: {e}", source.label());
                self.state.had_error = true;
                return;
            }
        };

        #[cfg(feature = "yaml")]
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
                    let _ = writeln!(self.err, "{NAME}: {}: parse error: {e}", source.label());
                    self.state.had_error = true;
                    continue;
                }
            };
            match_count += self.process_value(Some(source.label()), show_filename, value);
        }

        if !self.cli.slurp {
            if self.cli.count {
                if show_filename {
                    let _ = write!(self.out, "{}:", source.label());
                }
                let _ = writeln!(self.out, "{match_count}");
            } else if self.cli.files_with_matches && match_count > 0 {
                let _ = writeln!(self.out, "{}", source.label());
            }
        }
    }

    fn process_value(
        &mut self,
        filename: Option<String>,
        show_filename: bool,
        value: jaq_json::Val,
    ) -> usize {
        let results = match self.matcher.apply(value.clone()) {
            Ok(results) => results,
            Err(e) => {
                let _ = writeln!(self.err, "{NAME}: filter error: {e}");
                self.state.had_error = true;
                return 0;
            }
        };

        let mut count = 0usize;
        for result in results {
            if !matcher::is_match(&result) {
                continue;
            }
            count += 1;
            self.state.found_match = true;
            if self.cli.slurp {
                self.slurp.push(result);
            } else if !self.cli.count
                && !self.cli.files_with_matches
                && output::write_result(
                    self.out,
                    &result,
                    filename.as_deref(),
                    show_filename,
                    &value,
                    self.cli,
                )
                .is_err()
            {
                self.state.had_error = true;
            }
        }
        count
    }
}

#[derive(Default)]
struct RunState {
    found_match: bool,
    had_error: bool,
}

fn flush_output(out: &mut dyn Write, err: &mut dyn Write, state: &mut RunState) -> io::Result<()> {
    out.flush().inspect_err(|e| {
        let _ = writeln!(err, "{NAME}: error writing output: {e}");
        state.had_error = true;
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    struct FailingWriter;

    impl Write for FailingWriter {
        fn write(&mut self, _: &[u8]) -> io::Result<usize> {
            Err(io::Error::other("disk full"))
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn buffered_output_errors_are_reported() {
        let mut out = io::BufWriter::new(FailingWriter);
        let mut err = Vec::new();
        let mut state = RunState {
            found_match: true,
            had_error: false,
        };
        writeln!(out, "42").unwrap();

        assert!(flush_output(&mut out, &mut err, &mut state).is_err());
        assert!(state.had_error);
        assert!(String::from_utf8(err).unwrap().contains("disk full"));
    }
}
