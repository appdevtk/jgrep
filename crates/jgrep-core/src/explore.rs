use std::collections::VecDeque;
use std::io::{self, BufRead, IsTerminal, Read, Write};
use std::path::Path;
use std::sync::mpsc::{self, Receiver};
use std::thread;
use std::time::Duration;

use crossterm::event::{
    self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEventKind, KeyModifiers,
    MouseEvent, MouseEventKind,
};
use crossterm::execute;
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use jaq_json::Val;
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Constraint, Direction, Layout, Position, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, List, ListItem, Paragraph};
use ratatui::{Frame, Terminal};

use crate::cli::ExploreArgs;
use crate::input::{self, InputKind};
use crate::matcher::{self, Matcher};
use crate::schema::{self, FieldSummary};
use crate::{color, output, shortcuts, NAME};

pub fn run(args: ExploreArgs) -> i32 {
    let mut err = io::BufWriter::new(io::stderr());
    let config = ExploreConfig::from_args(&args);
    let filter = match initial_filter_input(&args) {
        Ok(filter) => filter,
        Err(e) => {
            let _ = writeln!(err, "{NAME}: explore: invalid where shortcut: {e}");
            return 2;
        }
    };
    let output_input = args.path_filter.clone().unwrap_or_default();
    let output_options = output_options_from_args(&args);

    if args.input.is_none()
        && !args.print
        && io::stdout().is_terminal()
        && !io::stdin().is_terminal()
    {
        if args.stream_json {
            let stream = spawn_json_line_stream(Vec::new(), config.max_input_bytes);
            let mut session =
                ExploreSession::new_streaming(filter, output_input, config, output_options);
            return finish_tui(run_tui(&mut session, Some(&stream)), &mut session, &mut err);
        }
        match prepare_stdin_tui(config.max_input_bytes) {
            Ok(StdinTuiInput::Streaming(first_line)) => {
                let stream = spawn_json_line_stream(first_line, config.max_input_bytes);
                let mut session =
                    ExploreSession::new_streaming(filter, output_input, config, output_options);
                return finish_tui(run_tui(&mut session, Some(&stream)), &mut session, &mut err);
            }
            Ok(StdinTuiInput::Buffered(bytes)) => {
                let values = match parse_values(input::detect_stdin_kind(&bytes), &bytes, true)
                    .map_err(|e| format!("stdin: {e}"))
                {
                    Ok(values) => values,
                    Err(e) => {
                        let _ = writeln!(err, "{NAME}: explore: {e}");
                        let _ = err.flush();
                        return 2;
                    }
                };
                let mut session =
                    ExploreSession::new(values, filter, output_input, config, output_options);
                return finish_tui(run_tui(&mut session, None), &mut session, &mut err);
            }
            Err(e) => {
                let _ = writeln!(err, "{NAME}: explore: {e}");
                let _ = err.flush();
                return 2;
            }
        }
    }

    let values = match read_values(args.input.as_deref(), config.max_input_bytes) {
        Ok(values) => values,
        Err(e) => {
            let _ = writeln!(err, "{NAME}: explore: {e}");
            let _ = err.flush();
            return 2;
        }
    };

    let mut session = ExploreSession::new(values, filter, output_input, config, output_options);
    if args.print || !io::stdout().is_terminal() {
        let mut out = io::BufWriter::new(io::stdout());
        let has_error = session.filter.error.is_some() || session.preview.cache.error_count > 0;
        if print_snapshot(&session, &mut out)
            .and_then(|()| out.flush())
            .is_err()
        {
            return 2;
        }
        return if has_error { 2 } else { 0 };
    }

    finish_tui(run_tui(&mut session, None), &mut session, &mut err)
}

fn initial_filter_input(args: &ExploreArgs) -> Result<String, String> {
    if args.where_filters.is_empty() {
        Ok(args.filter.clone().unwrap_or_default())
    } else {
        let filter = shortcuts::expression_to_jq(args.filter.as_deref().unwrap_or_default())?;
        shortcuts::apply_where_filters(filter, &args.where_filters)
    }
}

fn output_options_from_args(args: &ExploreArgs) -> output::OutputOptions {
    output::OutputOptions {
        pretty: args.pretty,
        json_color: false,
        color_level: args.color_level,
        no_color: args.no_color || std::env::var_os("NO_COLOR").is_some(),
        color_level_field: args.color_level_field.clone(),
    }
}

fn finish_tui(
    result: io::Result<ExplorerExit>,
    session: &mut ExploreSession,
    err: &mut dyn Write,
) -> i32 {
    match result {
        Ok(ExplorerExit::PrintResults) => print_results(session),
        Ok(ExplorerExit::PrintFilter) => {
            session.record_history();
            println!("{}", session.filter.generated);
            0
        }
        Ok(ExplorerExit::Quit) => 0,
        Err(e) => {
            let _ = writeln!(err, "{NAME}: explore: {e}");
            let _ = err.flush();
            2
        }
    }
}

enum StdinTuiInput {
    Streaming(Vec<u8>),
    Buffered(Vec<u8>),
}

fn prepare_stdin_tui(max_input_bytes: usize) -> Result<StdinTuiInput, String> {
    let stdin = io::stdin();
    // Stdin owns a persistent buffer. An extra BufReader would discard any
    // read-ahead when this function hands stdin to the streaming thread.
    let mut reader = stdin.lock();
    let mut first_line = Vec::new();
    reader
        .by_ref()
        .take((max_input_bytes as u64).saturating_add(1))
        .read_until(b'\n', &mut first_line)
        .map_err(|e| format!("stdin: {e}"))?;

    if first_line.len() > max_input_bytes {
        return Err(format!(
            "stdin: input is larger than --max-input-bytes ({max_input_bytes} bytes)"
        ));
    }

    if input::is_streamable_json_line(&first_line) {
        return Ok(StdinTuiInput::Streaming(first_line));
    }

    let mut bytes = Vec::with_capacity(max_input_bytes.min(1024 * 1024));
    bytes.extend_from_slice(&first_line);
    reader
        .take((max_input_bytes.saturating_sub(bytes.len()) as u64).saturating_add(1))
        .read_to_end(&mut bytes)
        .map_err(|e| format!("stdin: {e}"))?;
    if bytes.len() > max_input_bytes {
        return Err(format!(
            "stdin: input is larger than --max-input-bytes ({max_input_bytes} bytes)"
        ));
    }
    Ok(StdinTuiInput::Buffered(bytes))
}

enum StreamEvent {
    Line(Vec<u8>),
    Error(String),
    End,
}

fn spawn_json_line_stream(first_line: Vec<u8>, max_input_bytes: usize) -> Receiver<StreamEvent> {
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        let stdin = io::stdin();
        let mut reader = io::BufReader::new(stdin.lock());
        let mut bytes_read = 0usize;
        send_stream_line(&tx, &first_line, &mut bytes_read, max_input_bytes);
        if bytes_read > max_input_bytes {
            let _ = tx.send(StreamEvent::End);
            return;
        }

        let mut line = Vec::new();
        loop {
            line.clear();
            match reader
                .by_ref()
                .take((max_input_bytes.saturating_sub(bytes_read) as u64).saturating_add(1))
                .read_until(b'\n', &mut line)
            {
                Ok(0) => break,
                Ok(_) => send_stream_line(&tx, &line, &mut bytes_read, max_input_bytes),
                Err(e) => {
                    let _ = tx.send(StreamEvent::Error(format!("stdin: {e}")));
                    break;
                }
            }
            if bytes_read > max_input_bytes {
                break;
            }
        }
        let _ = tx.send(StreamEvent::End);
    });
    rx
}

fn send_stream_line(
    tx: &mpsc::Sender<StreamEvent>,
    line: &[u8],
    bytes_read: &mut usize,
    max_input_bytes: usize,
) {
    *bytes_read = bytes_read.saturating_add(line.len());
    if *bytes_read > max_input_bytes {
        let _ = tx.send(StreamEvent::Error(format!(
            "stdin: input is larger than --max-input-bytes ({max_input_bytes} bytes)"
        )));
        return;
    }

    let trimmed = input::trim_ascii_whitespace(line);
    if trimmed.is_empty() {
        return;
    }
    let _ = tx.send(StreamEvent::Line(trimmed.to_vec()));
}

#[derive(Debug, Clone, Copy)]
struct ExploreConfig {
    max_input_bytes: usize,
    max_schema_documents: usize,
    max_preview_results: usize,
    max_schema_depth: usize,
}

impl ExploreConfig {
    fn from_args(args: &ExploreArgs) -> Self {
        Self {
            max_input_bytes: args.max_input_bytes,
            max_schema_documents: args.max_schema_documents,
            max_preview_results: args.max_preview_results,
            max_schema_depth: args.max_schema_depth,
        }
    }
}

pub fn read_values(path: Option<&Path>, max_input_bytes: usize) -> Result<Vec<Val>, String> {
    match path {
        Some(path) => {
            if std::fs::metadata(path)
                .map_err(|e| format!("{}: {e}", path.display()))?
                .len()
                > max_input_bytes as u64
            {
                return Err(format!(
                    "{}: input is larger than --max-input-bytes ({max_input_bytes} bytes)",
                    path.display()
                ));
            }
            let mut bytes = Vec::with_capacity(max_input_bytes.min(1024 * 1024));
            std::fs::File::open(path)
                .and_then(|file| {
                    file.take((max_input_bytes as u64).saturating_add(1))
                        .read_to_end(&mut bytes)
                })
                .map_err(|e| format!("{}: {e}", path.display()))?;
            if bytes.len() > max_input_bytes {
                return Err(format!(
                    "{}: input is larger than --max-input-bytes ({max_input_bytes} bytes)",
                    path.display()
                ));
            }
            let kind = input::kind_for_path(path).unwrap_or(InputKind::Json);
            let allow_yaml_fallback = input::kind_for_path(path).is_none();
            parse_values(kind, &bytes, allow_yaml_fallback)
                .map_err(|e| format!("{}: {e}", path.display()))
        }
        None => {
            if io::stdin().is_terminal() {
                return Err("provide a file path or pipe JSON, NDJSON, or YAML to stdin".to_owned());
            }
            let mut bytes = Vec::with_capacity(max_input_bytes.min(1024 * 1024));
            io::stdin()
                .take((max_input_bytes as u64).saturating_add(1))
                .read_to_end(&mut bytes)
                .map_err(|e| format!("stdin: {e}"))?;
            if bytes.len() > max_input_bytes {
                return Err(format!(
                    "stdin: input is larger than --max-input-bytes ({max_input_bytes} bytes)"
                ));
            }
            parse_values(input::detect_stdin_kind(&bytes), &bytes, true)
                .map_err(|e| format!("stdin: {e}"))
        }
    }
}

fn parse_values(
    kind: InputKind,
    bytes: &[u8],
    allow_yaml_fallback: bool,
) -> Result<Vec<Val>, String> {
    let parsed = input::parse_many(kind, bytes)?;
    #[cfg(not(feature = "yaml"))]
    let _ = allow_yaml_fallback;
    #[cfg(feature = "yaml")]
    if allow_yaml_fallback && kind == InputKind::Json && parsed.first().is_some_and(Result::is_err)
    {
        if let Ok(values) = input::parse_many(InputKind::Yaml, bytes) {
            return collect_values(values);
        }
    }
    collect_values(parsed)
}

fn collect_values(values: Vec<Result<Val, String>>) -> Result<Vec<Val>, String> {
    values.into_iter().collect::<Result<Vec<_>, _>>()
}

fn compose_explore_jq(filter_input: &str, output_input: &str) -> Result<String, String> {
    let filter_input = filter_input.trim();
    let output_input = output_input.trim();

    if output_input.is_empty() {
        return shortcuts::expression_to_jq(filter_input);
    }

    let output = shortcuts::output_expression_to_jq(output_input)?;
    if filter_input.is_empty() {
        return Ok(output);
    }

    let filter = shortcuts::expression_to_jq(filter_input)?;
    let filter = filter.strip_suffix(" | .").unwrap_or(&filter);
    Ok(format!("{filter} | {output}"))
}

struct ExplorerData {
    values: Vec<Val>,
    fields: Vec<FieldSummary>,
}

struct TextState {
    input: String,
    cursor: usize,
}

struct FilterState {
    text: TextState,
    generated: String,
    error: Option<String>,
    history: Vec<String>,
    history_index: Option<usize>,
    completions: Vec<FieldCompletion>,
    completion_index: usize,
    completion_prefix: String,
}

#[derive(Clone)]
struct FieldCompletion {
    label: String,
    expression: String,
    types: String,
}

struct PreviewState {
    lines: Vec<PreviewLine>,
    match_count: usize,
    limit: usize,
    field_scroll: usize,
    preview_scroll: usize,
    cache: TuiPreview,
    follow_tail: bool,
    visual_lines: Vec<Line<'static>>,
    viewport_width: u16,
    viewport_height: u16,
    displayed_results: usize,
}

#[derive(Clone)]
struct PreviewLine {
    text: String,
    style: Style,
    badge: Option<PreviewBadge>,
}

#[derive(Clone, Copy)]
struct PreviewBadge {
    label: &'static str,
    style: Style,
}

struct ExplorerPersistence {
    last_filter_file: Option<std::ffi::OsString>,
}

#[derive(Default)]
struct HelpState {
    scroll: u16,
    max_scroll: u16,
}

impl HelpState {
    fn scroll(&mut self, delta: i16) {
        self.scroll = self
            .scroll
            .saturating_add_signed(delta)
            .min(self.max_scroll);
    }
}

const EXPLORER_HELP: &str = "Filter selects input records.\n\
Output chooses each result.\n\
Example: Filter status=active, Output name\n\n\
Editing\n\
Ctrl-O: switch Filter / Output\n\
Tab: complete an observed field path\n\
Left/Right, Home/End: move cursor\n\
Backspace/Delete: remove characters\n\
Ctrl-P/N: previous/next filter history\n\n\
Results and saving\n\
Enter: print results and exit\n\
Ctrl-Y: print generated jq and exit\n\
Ctrl-S: save generated jq to a file\n\
Default save file: jgrep-filter.jq\n\
Esc or Ctrl-C: quit without printing\n\n\
Preview\n\
Ctrl-G: toggle full-width preview\n\
Ctrl-F: toggle pretty formatting\n\
Ctrl-L: toggle log-level colors\n\
Ctrl-K: toggle all colors\n\
Ctrl-T: pause/resume live tail\n\
PgUp/PgDn or mouse wheel: scroll preview\n\
Up/Down: visible fields in Filter,\n\
otherwise scroll the preview\n\
Below 80 columns, fields are hidden\n\
automatically to leave room for results.\n\n\
While help is open, editing and output\n\
shortcuts are paused. F1 or Esc returns\n\
to your unchanged filter and preview.";

struct ExploreSession {
    data: ExplorerData,
    filter: FilterState,
    output: TextState,
    active_input: ActiveInput,
    preview_fullscreen: bool,
    narrow_terminal: bool,
    help: Option<HelpState>,
    output_options: output::OutputOptions,
    preview: PreviewState,
    persistence: ExplorerPersistence,
    config: ExploreConfig,
    stream: Option<StreamState>,
    message: Option<String>,
    compiled_filter: Option<(String, Matcher)>,
    defer_refresh: bool,
    refresh_pending: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ActiveInput {
    Filter,
    Output,
}

struct StreamState {
    ended: bool,
    errors: Vec<String>,
}

impl ExploreSession {
    fn new(
        values: Vec<Val>,
        filter: String,
        output_input: String,
        config: ExploreConfig,
        output_options: output::OutputOptions,
    ) -> Self {
        let (fields, schema_error) = match schema::infer_schema(
            &values,
            config.max_schema_documents,
            config.max_schema_depth,
        ) {
            Ok(fields) => (fields, None),
            Err(error) => (Vec::new(), Some(error)),
        };
        let cursor = filter.len();
        let mut session = Self {
            data: ExplorerData { values, fields },
            filter: FilterState {
                text: TextState {
                    input: filter,
                    cursor,
                },
                generated: String::new(),
                error: None,
                history: load_history(),
                history_index: None,
                completions: Vec::new(),
                completion_index: 0,
                completion_prefix: String::new(),
            },
            output: TextState {
                cursor: output_input.len(),
                input: output_input,
            },
            active_input: ActiveInput::Filter,
            preview_fullscreen: false,
            narrow_terminal: false,
            help: None,
            output_options,
            preview: PreviewState {
                lines: Vec::new(),
                match_count: 0,
                limit: config.max_preview_results,
                field_scroll: 0,
                preview_scroll: 0,
                cache: TuiPreview::default(),
                follow_tail: true,
                visual_lines: Vec::new(),
                viewport_width: 0,
                viewport_height: 0,
                displayed_results: 0,
            },
            persistence: ExplorerPersistence {
                last_filter_file: std::env::var_os("JGREP_LAST_FILTER_FILE"),
            },
            config,
            stream: None,
            message: schema_error,
            compiled_filter: None,
            defer_refresh: false,
            refresh_pending: false,
        };
        session.refresh();
        session
    }

    fn new_streaming(
        filter: String,
        output_input: String,
        config: ExploreConfig,
        output_options: output::OutputOptions,
    ) -> Self {
        let mut session = Self::new(Vec::new(), filter, output_input, config, output_options);
        session.stream = Some(StreamState {
            ended: false,
            errors: Vec::new(),
        });
        session.message = None;
        session
    }

    fn append_stream_values(&mut self, values: Vec<Val>) {
        let previous_len = self.data.values.len();
        self.data.values.extend(values);
        if previous_len < self.config.max_schema_documents {
            match schema::infer_schema(
                &self.data.values,
                self.config.max_schema_documents,
                self.config.max_schema_depth,
            ) {
                Ok(fields) => self.data.fields = fields,
                Err(error) => {
                    self.data.fields.clear();
                    self.message = Some(error);
                }
            }
        }
        if self.filter.error.is_none() {
            if let Some((_, matcher)) = &self.compiled_filter {
                extend_tui_preview(
                    &mut self.preview.cache,
                    &self.data.values[previous_len..],
                    matcher,
                    self.preview.limit,
                    &self.output_options,
                    true,
                );
                self.sync_preview_lines();
                return;
            }
        }
        self.refresh();
    }

    fn record_stream_error(&mut self, error: String) {
        if let Some(stream) = &mut self.stream {
            stream.errors.push(error);
        }
    }

    fn end_stream(&mut self) {
        self.preview.viewport_width = 0;
        if let Some(stream) = &mut self.stream {
            stream.ended = true;
        }
    }

    fn refresh(&mut self) {
        if self.defer_refresh {
            self.refresh_pending = true;
            return;
        }
        self.refresh_pending = false;
        match compose_explore_jq(&self.filter.text.input, &self.output.input) {
            Ok(generated) => {
                self.filter.generated = generated;
                let changed = self
                    .compiled_filter
                    .as_ref()
                    .is_none_or(|(code, _)| code != &self.filter.generated);
                let preview = (|| {
                    if self
                        .compiled_filter
                        .as_ref()
                        .is_none_or(|(code, _)| code != &self.filter.generated)
                    {
                        self.compiled_filter = Some((
                            self.filter.generated.clone(),
                            Matcher::compile(&self.filter.generated)?,
                        ));
                    }
                    let (_, matcher) = self.compiled_filter.as_ref().unwrap();
                    Ok(preview_tui_values(
                        &self.data.values,
                        matcher,
                        self.preview.limit,
                        &self.output_options,
                        self.stream.is_some(),
                    ))
                })();
                match preview {
                    Ok(preview) => {
                        self.preview.cache = preview;
                        self.preview.follow_tail = true;
                        self.sync_preview_lines();
                        self.filter.error = None;
                        if changed {
                            self.persist_last_filter();
                        }
                    }
                    Err(e) => {
                        self.preview.lines.clear();
                        self.preview.viewport_width = 0;
                        self.preview.displayed_results = 0;
                        self.preview.match_count = 0;
                        self.preview.cache = TuiPreview::default();
                        self.filter.error = Some(e);
                    }
                }
            }
            Err(e) => {
                self.filter.generated.clear();
                self.preview.lines.clear();
                self.preview.viewport_width = 0;
                self.preview.displayed_results = 0;
                self.preview.match_count = 0;
                self.preview.cache = TuiPreview::default();
                self.filter.error = Some(e);
            }
        }
    }

    fn flush_pending_refresh(&mut self) {
        if !self.refresh_pending {
            return;
        }
        let deferred = self.defer_refresh;
        self.defer_refresh = false;
        self.refresh();
        self.defer_refresh = deferred;
    }

    fn sync_preview_lines(&mut self) {
        self.preview.match_count = self.preview.cache.match_count;
        if self.stream.is_none() || self.preview.follow_tail {
            self.preview.viewport_width = 0;
            self.preview.displayed_results = self.preview.cache.results.len();
            self.preview.lines = self
                .preview
                .cache
                .results
                .iter()
                .flatten()
                .cloned()
                .collect();
            self.clamp_preview_scroll();
        }
    }

    fn toggle_follow_tail(&mut self) {
        if self.stream.is_none() {
            return;
        }
        self.preview.follow_tail = !self.preview.follow_tail;
        self.sync_preview_lines();
        self.preview.viewport_width = 0;
        // The persistent tail/paused status is clearer than a notice that hides
        // that status in narrow terminals.
        self.message = None;
    }

    fn autocomplete_filter(&mut self) {
        let active_input = self.active_text().input.clone();
        if !self.filter.completions.is_empty()
            && self
                .filter
                .completions
                .get(self.filter.completion_index)
                .is_some_and(|candidate| candidate.expression == active_input)
        {
            self.filter.completion_index =
                (self.filter.completion_index + 1) % self.filter.completions.len();
            let replacement = self.filter.completions[self.filter.completion_index]
                .expression
                .clone();
            self.active_text_mut().input = replacement;
            self.active_text_mut().cursor = self.active_text().input.len();
            self.message = Some(self.completion_message());
            self.refresh();
            return;
        }

        let prefix = self.active_text().input.trim().to_owned();
        if prefix.is_empty() || prefix.starts_with('.') || prefix.starts_with("select(") {
            return;
        }

        self.filter.completions = self
            .data
            .fields
            .iter()
            .filter(|field| field.path.starts_with(&prefix))
            .map(|field| FieldCompletion {
                label: field.path.clone(),
                expression: field.jq_path.clone(),
                types: field.types.join("|"),
            })
            .collect();
        self.filter.completion_index = 0;
        self.filter.completion_prefix = prefix;

        if let Some(field) = self.filter.completions.first() {
            self.active_text_mut().input = field.expression.clone();
            self.active_text_mut().cursor = self.active_text().input.len();
            self.message = Some(self.completion_message());
            self.refresh();
        }
    }

    fn completion_message(&self) -> String {
        let total = self.filter.completions.len();
        let index = self.filter.completion_index + 1;
        let current = self
            .filter
            .completions
            .get(self.filter.completion_index)
            .map(|field| field.label.as_str())
            .unwrap_or("");
        let candidates = self
            .filter
            .completions
            .iter()
            .take(5)
            .map(|field| {
                if field.types.is_empty() {
                    field.label.clone()
                } else {
                    format!("{} [{}]", field.label, field.types)
                }
            })
            .collect::<Vec<_>>()
            .join(", ");
        format!("completion {index}/{total}: {current} | {candidates}")
    }

    fn save_filter(&mut self) {
        if self.filter.generated.is_empty() || self.filter.error.is_some() {
            self.message = Some("no valid filter to save".to_owned());
            return;
        }

        let path = std::env::var("JGREP_FILTER_SAVE_PATH")
            .unwrap_or_else(|_| "jgrep-filter.jq".to_owned());
        match std::fs::write(&path, format!("{}\n", self.filter.generated)) {
            Ok(()) => {
                self.record_history();
                self.message = Some(format!("saved {path}"));
            }
            Err(e) => self.message = Some(format!("save failed: {e}")),
        }
    }

    fn persist_last_filter(&mut self) {
        let Some(path) = &self.persistence.last_filter_file else {
            return;
        };
        if let Err(e) = std::fs::write(path, format!("{}\n", self.filter.generated)) {
            self.message = Some(format!("last-filter write failed: {e}"));
        }
    }

    fn record_history(&mut self) {
        let input = self.filter.text.input.trim();
        if input.is_empty() || self.filter.history.last().is_some_and(|last| last == input) {
            return;
        }
        self.filter.history.push(input.to_owned());
        if let Some(path) = std::env::var_os("JGREP_FILTER_HISTORY_FILE") {
            let _ = std::fs::write(path, format!("{}\n", self.filter.history.join("\n")));
        }
    }

    fn load_history_entry(&mut self, previous: bool) {
        if self.filter.history.is_empty() {
            return;
        }
        let current = self
            .filter
            .history_index
            .unwrap_or(self.filter.history.len());
        let next = if previous {
            current.saturating_sub(1)
        } else {
            (current + 1).min(self.filter.history.len())
        };
        if next >= self.filter.history.len() {
            self.filter.history_index = None;
            return;
        }
        self.filter.history_index = Some(next);
        self.filter.text.input = self.filter.history[next].clone();
        self.filter.text.cursor = self.filter.text.input.len();
        self.active_input = ActiveInput::Filter;
        self.message = Some(format!(
            "history {}/{}",
            next + 1,
            self.filter.history.len()
        ));
        self.refresh();
    }

    fn insert_char(&mut self, ch: char) {
        self.clear_completion_state();
        let text = self.active_text_mut();
        text.input.insert(text.cursor, ch);
        text.cursor += ch.len_utf8();
        self.refresh();
    }

    fn backspace(&mut self) {
        self.clear_completion_state();
        let text = self.active_text_mut();
        if text.cursor == 0 {
            return;
        }
        let previous = previous_boundary(&text.input, text.cursor);
        text.input.drain(previous..text.cursor);
        text.cursor = previous;
        self.refresh();
    }

    fn delete(&mut self) {
        self.clear_completion_state();
        let text = self.active_text_mut();
        if text.cursor >= text.input.len() {
            return;
        }
        let next = next_boundary(&text.input, text.cursor);
        text.input.drain(text.cursor..next);
        self.refresh();
    }

    fn move_cursor_left(&mut self) {
        let text = self.active_text_mut();
        text.cursor = previous_boundary(&text.input, text.cursor);
    }

    fn move_cursor_right(&mut self) {
        let text = self.active_text_mut();
        text.cursor = next_boundary(&text.input, text.cursor);
    }

    fn active_text(&self) -> &TextState {
        match self.active_input {
            ActiveInput::Filter => &self.filter.text,
            ActiveInput::Output => &self.output,
        }
    }

    fn active_text_mut(&mut self) -> &mut TextState {
        match self.active_input {
            ActiveInput::Filter => &mut self.filter.text,
            ActiveInput::Output => &mut self.output,
        }
    }

    fn toggle_active_input(&mut self) {
        self.clear_completion_state();
        self.active_input = match self.active_input {
            ActiveInput::Filter => ActiveInput::Output,
            ActiveInput::Output => ActiveInput::Filter,
        };
        self.message = Some(format!(
            "editing {}",
            match self.active_input {
                ActiveInput::Filter => "filter",
                ActiveInput::Output => "output",
            }
        ));
    }

    fn toggle_preview_fullscreen(&mut self) {
        self.preview_fullscreen = !self.preview_fullscreen;
        self.message = Some(format!(
            "logs {}",
            if self.preview_fullscreen {
                "fullscreen"
            } else {
                "split"
            }
        ));
    }

    fn toggle_pretty(&mut self) {
        self.output_options.pretty = !self.output_options.pretty;
        self.message = Some(format!(
            "pretty {}",
            if self.output_options.pretty {
                "on"
            } else {
                "off"
            }
        ));
        self.refresh();
    }

    fn toggle_level_color(&mut self) {
        self.output_options.color_level = !self.output_options.color_level;
        self.output_options.no_color = false;
        self.message = Some(format!(
            "level color {}",
            if self.output_options.color_level {
                "on"
            } else {
                "off"
            }
        ));
        self.refresh();
    }

    fn toggle_no_color(&mut self) {
        self.output_options.no_color = !self.output_options.no_color;
        self.message = Some(format!(
            "color {}",
            if self.output_options.no_color {
                "off"
            } else {
                "on"
            }
        ));
        self.refresh();
    }

    fn clear_completion_state(&mut self) {
        self.message = None;
        self.filter.history_index = None;
        self.filter.completions.clear();
        self.filter.completion_index = 0;
        self.filter.completion_prefix.clear();
    }

    fn scroll_fields(&mut self, delta: isize) {
        self.preview.field_scroll =
            saturating_offset(self.preview.field_scroll, delta, self.data.fields.len());
    }

    fn scroll_preview(&mut self, delta: isize) {
        if self.stream.is_some() && delta < 0 {
            self.preview.follow_tail = false;
            if self.preview.lines.is_empty() {
                self.preview.viewport_width = 0;
            }
        }
        let max = self.max_preview_scroll();
        let next = self.preview.preview_scroll.saturating_add_signed(delta);
        self.preview.preview_scroll = next.min(max);
    }

    fn clamp_preview_scroll(&mut self) {
        let max = self.max_preview_scroll();
        self.preview.preview_scroll = self.preview.preview_scroll.min(max);
    }

    fn max_preview_scroll(&self) -> usize {
        let (rows, height) = if self.preview.viewport_width == 0 {
            (self.preview.lines.len(), 1)
        } else {
            (
                self.preview.visual_lines.len(),
                usize::from(self.preview.viewport_height.max(1)),
            )
        };
        rows.saturating_sub(height)
    }

    fn status_line(&self) -> String {
        let active = match self.active_input {
            ActiveInput::Filter => "filter",
            ActiveInput::Output => "output",
        };
        let pretty = if self.output_options.pretty {
            "pretty:on"
        } else {
            "pretty:off"
        };
        let level = if self.output_options.color_level && !self.output_options.no_color {
            "color:on"
        } else {
            "color:off"
        };
        let scroll = if self.preview.lines.is_empty() {
            "scroll:0/0".to_owned()
        } else {
            format!(
                "scroll:{}/{}",
                self.preview.preview_scroll.saturating_add(1),
                if self.preview.viewport_width == 0 {
                    self.preview.lines.len()
                } else {
                    self.preview.visual_lines.len()
                }
            )
        };
        let layout = if self.preview_fullscreen {
            "logs:full"
        } else {
            "logs:split"
        };
        let notice = self
            .message
            .as_ref()
            .map(|message| format!(" | {message}"))
            .unwrap_or_default();
        format!(
            "{}{notice} | {active} | {pretty} | {level} | {layout} | {scroll}",
            self.preview_status()
        )
    }

    fn preview_status(&self) -> String {
        let mode = if self.stream.is_some() {
            if self.preview.follow_tail {
                "tail"
            } else {
                "paused"
            }
        } else {
            "first"
        };
        format!(
            "{} matches | {mode}:{} | {} eval errors",
            self.preview.match_count,
            self.preview.displayed_results,
            self.preview.cache.error_count
        )
    }
}

fn load_history() -> Vec<String> {
    let Some(path) = std::env::var_os("JGREP_FILTER_HISTORY_FILE") else {
        return Vec::new();
    };
    std::fs::read_to_string(path)
        .map(|history| {
            history
                .lines()
                .map(str::trim)
                .filter(|line| !line.is_empty())
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default()
}

fn previous_boundary(input: &str, cursor: usize) -> usize {
    let line = Line::raw(&input[..cursor]);
    let mut offset = 0;
    let mut previous = 0;
    for grapheme in line.styled_graphemes(Style::default()) {
        previous = offset;
        offset += grapheme.symbol.len();
    }
    previous
}

fn next_boundary(input: &str, cursor: usize) -> usize {
    let line = Line::raw(&input[cursor..]);
    let next = cursor
        + line
            .styled_graphemes(Style::default())
            .next()
            .map_or(0, |grapheme| grapheme.symbol.len());
    next
}

fn saturating_offset(current: usize, delta: isize, len: usize) -> usize {
    if delta < 0 {
        current.saturating_sub(delta.unsigned_abs())
    } else {
        current
            .saturating_add(delta as usize)
            .min(len.saturating_sub(1))
    }
}

#[cfg(test)]
fn preview_values(
    values: &[Val],
    filter: &str,
    limit: usize,
    options: &output::OutputOptions,
) -> Result<Vec<String>, String> {
    preview_values_for(values, filter, limit, options, PreviewTarget::Raw)
}

enum PreviewTarget {
    #[cfg(test)]
    Raw,
    Terminal,
}

fn preview_values_for(
    values: &[Val],
    filter: &str,
    limit: usize,
    options: &output::OutputOptions,
    target: PreviewTarget,
) -> Result<Vec<String>, String> {
    let matcher = Matcher::compile(filter)?;
    let mut lines = Vec::new();
    if limit == 0 {
        return Ok(lines);
    }

    for value in values {
        let Ok(results) = matcher.apply(value.clone()) else {
            continue;
        };
        for result in results {
            if !matcher::is_match(&result) {
                continue;
            }
            let text = match target {
                #[cfg(test)]
                PreviewTarget::Raw => output::format_result(&result, value, options),
                PreviewTarget::Terminal => output::format_terminal_result(&result, value, options),
            };
            if let Ok(text) = text {
                lines.push(text);
            }
            if lines.len() >= limit {
                return Ok(lines);
            }
        }
    }

    Ok(lines)
}

#[derive(Default)]
struct TuiPreview {
    results: VecDeque<Vec<PreviewLine>>,
    match_count: usize,
    error_count: usize,
    first_error: Option<String>,
}

fn preview_tui_values(
    values: &[Val],
    matcher: &Matcher,
    limit: usize,
    options: &output::OutputOptions,
    tail: bool,
) -> TuiPreview {
    let mut preview = TuiPreview::default();
    extend_tui_preview(&mut preview, values, matcher, limit, options, tail);
    preview
}

fn extend_tui_preview(
    preview: &mut TuiPreview,
    values: &[Val],
    matcher: &Matcher,
    limit: usize,
    options: &output::OutputOptions,
    tail: bool,
) {
    let display_options = output::OutputOptions {
        pretty: options.pretty,
        json_color: false,
        color_level: false,
        no_color: true,
        color_level_field: options.color_level_field.clone(),
    };

    for value in values {
        let results = match matcher.apply(value.clone()) {
            Ok(results) => results,
            Err(error) => {
                preview.error_count += 1;
                preview.first_error.get_or_insert(error);
                continue;
            }
        };
        for result in results {
            if !matcher::is_match(&result) {
                continue;
            }
            preview.match_count += 1;
            if limit == 0 || (!tail && preview.results.len() >= limit) {
                continue;
            }
            let text = match output::format_terminal_result(&result, value, &display_options) {
                Ok(text) => text,
                Err(error) => {
                    preview.error_count += 1;
                    preview.first_error.get_or_insert(error.to_string());
                    continue;
                }
            };
            let (style, badge) = preview_line_style(value, options);
            let mut lines = Vec::new();
            for line in text.lines() {
                lines.push(PreviewLine {
                    text: line.to_owned(),
                    style,
                    badge,
                });
            }
            if text.is_empty() {
                lines.push(PreviewLine {
                    text: String::new(),
                    style,
                    badge,
                });
            }
            preview.results.push_back(lines);
            if preview.results.len() > limit {
                preview.results.pop_front();
            }
        }
    }
}

fn preview_line_style(
    source: &Val,
    options: &output::OutputOptions,
) -> (Style, Option<PreviewBadge>) {
    match color::log_level_without_env(
        source,
        options.color_level,
        options.no_color,
        options.color_level_field.as_deref(),
    ) {
        Some(color::LogLevel::Info) => {
            level_preview_style("INFO", Style::default().fg(Color::Rgb(80, 220, 235)))
        }
        Some(color::LogLevel::Debug) => {
            level_preview_style("DEBUG", Style::default().fg(Color::Rgb(120, 170, 255)))
        }
        Some(color::LogLevel::Warn) => {
            level_preview_style("WARN", Style::default().fg(Color::Rgb(255, 205, 80)))
        }
        Some(color::LogLevel::Trace) => {
            level_preview_style("TRACE", Style::default().fg(Color::Rgb(235, 120, 255)))
        }
        Some(color::LogLevel::Error) => {
            level_preview_style("ERROR", Style::default().fg(Color::Rgb(255, 80, 90)))
        }
        Some(color::LogLevel::Fatal) => level_preview_style(
            "FATAL",
            Style::default()
                .fg(Color::Rgb(255, 80, 90))
                .add_modifier(Modifier::BOLD),
        ),
        None => (Style::default(), None),
    }
}

fn level_preview_style(label: &'static str, style: Style) -> (Style, Option<PreviewBadge>) {
    (
        style,
        Some(PreviewBadge {
            label,
            style: style.add_modifier(Modifier::BOLD),
        }),
    )
}

fn preview_render_lines(session: &ExploreSession, width: u16) -> Vec<Line<'static>> {
    if let Some(error) = &session.filter.error {
        return wrap_text(&format!("error: {}", output::escape_terminal(error)), width)
            .into_iter()
            .map(|line| Line::styled(line, Style::default().fg(Color::Red)))
            .collect();
    }
    if session.preview.lines.is_empty() {
        let message = if session.preview.limit == 0 {
            "preview disabled (--max-preview-results 0)"
        } else if session.stream.is_some() && !session.preview.follow_tail {
            "preview paused; Ctrl-T resumes live logs"
        } else if session.stream.as_ref().is_some_and(|stream| !stream.ended) {
            "waiting for matching logs..."
        } else if session.preview.cache.error_count > 0 {
            "no compatible results; see evaluation errors"
        } else {
            "no matches; adjust Filter or Output (F1 help)"
        };
        return wrap_text(message, width)
            .into_iter()
            .map(Line::raw)
            .collect();
    }

    session
        .preview
        .lines
        .iter()
        .flat_map(|line| {
            let badge = line.badge.filter(|_| width >= 10);
            let text_width = width.saturating_sub(if badge.is_some() { 7 } else { 0 });
            wrap_text(&line.text, text_width)
                .into_iter()
                .enumerate()
                .map(move |(index, text)| {
                    let text = if line.style.bg.is_some() {
                        pad_preview_line(&text, text_width)
                    } else {
                        text
                    };
                    let mut spans = Vec::new();
                    if let Some(badge) = badge {
                        let label = if index == 0 {
                            format!(" {:<5} ", badge.label)
                        } else {
                            "       ".to_owned()
                        };
                        spans.push(Span::styled(label, badge.style));
                    }
                    spans.push(Span::styled(text, line.style));
                    Line::from(spans)
                })
        })
        .collect()
}

// Wrap by terminal columns and whole graphemes, preserving JSON indentation.
// Rendering and scrolling use these same physical rows, including colored logs.
fn wrap_text(text: &str, width: u16) -> Vec<String> {
    if width == 0 {
        return Vec::new();
    }
    let source = Line::raw(text);
    let mut rows = Vec::new();
    let mut row = String::new();
    let mut columns = 0;
    for grapheme in source.styled_graphemes(Style::default()) {
        let size = Span::raw(grapheme.symbol).width();
        if columns + size > usize::from(width) && !row.is_empty() {
            rows.push(std::mem::take(&mut row));
            columns = 0;
        }
        if size > usize::from(width) {
            row.push('�');
            columns += 1;
        } else {
            row.push_str(grapheme.symbol);
            columns += size;
        }
    }
    if !row.is_empty() || rows.is_empty() {
        rows.push(row);
    }
    rows
}

fn pad_preview_line(text: &str, width: u16) -> String {
    let width = width as usize;
    let len = Line::raw(text).width();
    if len >= width {
        text.to_owned()
    } else {
        format!("{text}{}", " ".repeat(width - len))
    }
}

fn print_snapshot(session: &ExploreSession, out: &mut dyn Write) -> io::Result<()> {
    writeln!(out, "jgrep explore")?;
    if let Some(message) = &session.message {
        writeln!(out, "notice: {}", output::escape_terminal(message))?;
    }
    if let Some(error) = &session.preview.cache.first_error {
        writeln!(
            out,
            "notice: {} eval errors; skipped incompatible records: {}",
            session.preview.cache.error_count,
            output::escape_terminal(error)
        )?;
    }
    writeln!(
        out,
        "filter: {}",
        if session.filter.text.input.trim().is_empty() {
            ".".to_owned()
        } else {
            output::escape_terminal(session.filter.text.input.trim())
        }
    )?;
    writeln!(
        out,
        "output: {}",
        if session.output.input.trim().is_empty() {
            ".".to_owned()
        } else {
            output::escape_terminal(session.output.input.trim())
        }
    )?;
    writeln!(
        out,
        "generated jq: {}",
        output::escape_terminal(&session.filter.generated)
    )?;
    writeln!(out)?;
    writeln!(out, "fields:")?;
    for field in session.data.fields.iter().take(25) {
        let optional = if field.optional { " optional" } else { "" };
        let examples = if field.examples.is_empty() {
            String::new()
        } else {
            format!(
                " examples={}",
                field
                    .examples
                    .iter()
                    .map(|text| output::escape_terminal(text))
                    .collect::<Vec<_>>()
                    .join(",")
            )
        };
        writeln!(
            out,
            "  {}  {}  ({}, docs={}{}{examples})",
            output::escape_terminal(&field.path),
            field.types.join("|"),
            field.count,
            field.documents,
            optional,
        )?;
    }
    if session.data.fields.len() > 25 {
        writeln!(out, "  ... {} more", session.data.fields.len() - 25)?;
    }
    writeln!(out)?;
    writeln!(out, "preview:")?;
    if let Some(error) = &session.filter.error {
        writeln!(out, "  error: {}", output::escape_terminal(error))?;
    } else {
        let preview = preview_values_for(
            &session.data.values,
            &session.filter.generated,
            session.preview.limit,
            &session.output_options,
            PreviewTarget::Terminal,
        )
        .unwrap_or_default();
        if preview.is_empty() {
            writeln!(out, "  no matches")?;
        } else {
            for item in &preview {
                if item.is_empty() {
                    writeln!(out, "  ")?;
                } else {
                    for line in item.lines() {
                        writeln!(out, "  {line}")?;
                    }
                }
            }
        }
    }
    Ok(())
}

fn print_results(session: &ExploreSession) -> i32 {
    let mut out = io::BufWriter::new(io::stdout());
    let mut err = io::BufWriter::new(io::stderr());
    let code = write_explore_results(session, &mut out, &mut err);
    if out.flush().is_err() || err.flush().is_err() {
        return 2;
    }
    code
}

fn write_explore_results(
    session: &ExploreSession,
    out: &mut dyn Write,
    err: &mut dyn Write,
) -> i32 {
    if let Some(error) = &session.filter.error {
        let _ = writeln!(err, "{NAME}: explore: {error}");
        return 2;
    }
    let Some((_, matcher)) = &session.compiled_filter else {
        return 2;
    };
    let mut matches = 0;
    let mut errors = 0;
    let mut first_error = None;
    for value in &session.data.values {
        let results = match matcher.apply(value.clone()) {
            Ok(results) => results,
            Err(error) => {
                errors += 1;
                first_error.get_or_insert(error);
                continue;
            }
        };
        for result in results.into_iter().filter(matcher::is_match) {
            let line = match output::format_result(&result, value, &session.output_options) {
                Ok(line) => line,
                Err(error) => {
                    errors += 1;
                    first_error.get_or_insert(error.to_string());
                    continue;
                }
            };
            if let Err(error) = writeln!(out, "{line}") {
                let _ = writeln!(err, "{NAME}: explore: error writing output: {error}");
                return 2;
            }
            matches += 1;
        }
    }
    if let Some(error) = first_error {
        let _ = writeln!(
            err,
            "{NAME}: explore: {errors} eval errors; skipped incompatible records: {}",
            output::escape_terminal(&error)
        );
    }
    if let Some(stream) = &session.stream {
        if !stream.errors.is_empty() {
            for error in &stream.errors {
                let _ = writeln!(err, "{NAME}: explore: {}", output::escape_terminal(error));
            }
            return 2;
        }
    }
    if errors > 0 {
        2
    } else if matches == 0 {
        1
    } else {
        0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ExplorerExit {
    PrintResults,
    PrintFilter,
    Quit,
}

struct TerminalGuard {
    active: bool,
}

impl TerminalGuard {
    fn restore(&mut self) -> io::Result<()> {
        self.active = false;
        // Attempt both restoration operations, even if the first fails.
        let raw = disable_raw_mode();
        let screen = execute!(
            io::stdout(),
            DisableMouseCapture,
            LeaveAlternateScreen,
            crossterm::cursor::Show
        );
        raw.and(screen)
    }
}

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        if self.active {
            let _ = self.restore();
        }
    }
}

fn run_tui(
    session: &mut ExploreSession,
    stream: Option<&Receiver<StreamEvent>>,
) -> io::Result<ExplorerExit> {
    enable_raw_mode()?;
    // Install cleanup before any later initialization can return an error.
    let mut guard = TerminalGuard { active: true };
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let colors_disabled = crossterm::style::Colored::ansi_color_disabled_memoized();
    let result = run_tui_loop(&mut terminal, session, stream);
    crossterm::style::force_color_output(!colors_disabled);
    let restored = guard.restore();
    result.and_then(|exit| restored.map(|()| exit))
}

fn run_tui_loop(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    session: &mut ExploreSession,
    stream: Option<&Receiver<StreamEvent>>,
) -> io::Result<ExplorerExit> {
    let result = loop {
        let busy = stream.is_some_and(|stream| drain_stream(session, stream));

        let size = terminal.size()?;
        update_viewport(session, Rect::new(0, 0, size.width, size.height));
        crossterm::style::force_color_output(!session.output_options.no_color);
        terminal.draw(|frame| render(frame, session))?;

        let wait = if busy {
            Duration::ZERO
        } else {
            Duration::from_millis(150)
        };
        if !event::poll(wait)? {
            continue;
        }

        session.defer_refresh = true;
        let mut exit = None;
        for _ in 0..64 {
            match event::read()? {
                Event::Key(key) if key.kind == KeyEventKind::Press => {
                    exit = handle_key(session, key.code, key.modifiers);
                }
                Event::Mouse(mouse) => handle_mouse(session, mouse),
                _ => {}
            }
            // Allow the terminal reader to decode the rest of a paste/burst,
            // instead of rescanning the dataset after each arriving character.
            if exit.is_some() || !event::poll(Duration::from_millis(5))? {
                break;
            }
        }
        session.defer_refresh = false;
        session.flush_pending_refresh();
        if let Some(exit) = exit {
            break exit;
        }
    };
    Ok(result)
}

fn explorer_layout(area: Rect) -> [Rect; 3] {
    let outer = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(6),
            Constraint::Min(5),
            Constraint::Length(if area.width < 80 { 5 } else { 6 }),
        ])
        .split(area);
    [outer[0], outer[1], outer[2]]
}

fn wide_preview(area: Rect, preview_fullscreen: bool) -> bool {
    preview_fullscreen || area.width < 80
}

fn preview_area_for(area: Rect, preview_fullscreen: bool) -> Rect {
    let outer = explorer_layout(area);
    let body = if wide_preview(area, preview_fullscreen) {
        Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(100)])
            .split(outer[1])
    } else {
        Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(36), Constraint::Percentage(64)])
            .split(outer[1])
    };

    if wide_preview(area, preview_fullscreen) {
        body[0]
    } else {
        body[1]
    }
}

const STREAM_BATCH_SIZE: usize = 512;

fn drain_stream(session: &mut ExploreSession, stream: &Receiver<StreamEvent>) -> bool {
    let mut batch = Vec::new();
    let mut processed = 0;
    for event in stream.try_iter().take(STREAM_BATCH_SIZE) {
        processed += 1;
        match event {
            StreamEvent::Line(line) => match parse_stream_line(&line) {
                Ok(values) => {
                    batch.extend(values);
                }
                Err(error) => session.record_stream_error(error),
            },
            StreamEvent::Error(error) => session.record_stream_error(error),
            StreamEvent::End => {
                session.end_stream();
            }
        }
    }
    let added = !batch.is_empty();
    if added {
        session.append_stream_values(batch);
    }
    processed == STREAM_BATCH_SIZE
}

fn parse_stream_line(line: &[u8]) -> Result<Vec<Val>, String> {
    let values =
        input::parse_many(InputKind::Json, line).map_err(|e| format!("stdin: parse error: {e}"))?;
    values
        .into_iter()
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| format!("stdin: parse error: {e}"))
}

fn handle_key(
    session: &mut ExploreSession,
    code: KeyCode,
    modifiers: KeyModifiers,
) -> Option<ExplorerExit> {
    // Actions must observe all preceding edits, even within a pasted key batch.
    if code == KeyCode::Enter
        || (modifiers.contains(KeyModifiers::CONTROL) && matches!(code, KeyCode::Char('s' | 'y')))
    {
        session.flush_pending_refresh();
    }
    if code == KeyCode::F(1) {
        session.help = if session.help.is_some() {
            None
        } else {
            Some(HelpState::default())
        };
        return None;
    }
    if let Some(help) = &mut session.help {
        match code {
            KeyCode::Esc => session.help = None,
            KeyCode::Char('c') if modifiers.contains(KeyModifiers::CONTROL) => {
                return Some(ExplorerExit::Quit);
            }
            KeyCode::Up => help.scroll(-1),
            KeyCode::Down => help.scroll(1),
            KeyCode::PageUp => help.scroll(-10),
            KeyCode::PageDown => help.scroll(10),
            KeyCode::Home => help.scroll = 0,
            KeyCode::End => help.scroll = help.max_scroll,
            _ => {}
        }
        return None;
    }
    match code {
        KeyCode::Enter => {
            if session.filter.error.is_some() {
                session.message = Some("fix the filter before printing".to_owned());
                return None;
            }
            session.record_history();
            Some(ExplorerExit::PrintResults)
        }
        KeyCode::Esc => Some(ExplorerExit::Quit),
        KeyCode::Char('c') if modifiers.contains(KeyModifiers::CONTROL) => Some(ExplorerExit::Quit),
        KeyCode::Char('y') if modifiers.contains(KeyModifiers::CONTROL) => {
            if session.filter.error.is_some() {
                session.message = Some("no valid jq to export".to_owned());
                return None;
            }
            session.record_history();
            Some(ExplorerExit::PrintFilter)
        }
        KeyCode::Char('s') if modifiers.contains(KeyModifiers::CONTROL) => {
            session.save_filter();
            None
        }
        KeyCode::Char('o') if modifiers.contains(KeyModifiers::CONTROL) => {
            session.toggle_active_input();
            None
        }
        KeyCode::Char('g') if modifiers.contains(KeyModifiers::CONTROL) => {
            session.toggle_preview_fullscreen();
            None
        }
        KeyCode::Char('f') if modifiers.contains(KeyModifiers::CONTROL) => {
            session.toggle_pretty();
            None
        }
        KeyCode::Char('l') if modifiers.contains(KeyModifiers::CONTROL) => {
            session.toggle_level_color();
            None
        }
        KeyCode::Char('k') if modifiers.contains(KeyModifiers::CONTROL) => {
            session.toggle_no_color();
            None
        }
        KeyCode::Char('t') if modifiers.contains(KeyModifiers::CONTROL) => {
            session.toggle_follow_tail();
            None
        }
        KeyCode::Char('p') if modifiers.contains(KeyModifiers::CONTROL) => {
            session.load_history_entry(true);
            None
        }
        KeyCode::Char('n') if modifiers.contains(KeyModifiers::CONTROL) => {
            session.load_history_entry(false);
            None
        }
        KeyCode::Tab => {
            session.autocomplete_filter();
            None
        }
        KeyCode::Left => {
            session.move_cursor_left();
            None
        }
        KeyCode::Right => {
            session.move_cursor_right();
            None
        }
        KeyCode::Home => {
            session.active_text_mut().cursor = 0;
            None
        }
        KeyCode::End => {
            session.active_text_mut().cursor = session.active_text().input.len();
            None
        }
        KeyCode::Delete => {
            session.delete();
            None
        }
        KeyCode::Up => {
            if session.preview_fullscreen
                || session.narrow_terminal
                || session.active_input == ActiveInput::Output
            {
                session.scroll_preview(-1);
            } else {
                session.scroll_fields(-1);
            }
            None
        }
        KeyCode::Down => {
            if session.preview_fullscreen
                || session.narrow_terminal
                || session.active_input == ActiveInput::Output
            {
                session.scroll_preview(1);
            } else {
                session.scroll_fields(1);
            }
            None
        }
        KeyCode::PageUp => {
            let page = if session.preview.viewport_height == 0 {
                10
            } else {
                session.preview.viewport_height as isize
            };
            session.scroll_preview(-page);
            None
        }
        KeyCode::PageDown => {
            let page = if session.preview.viewport_height == 0 {
                10
            } else {
                session.preview.viewport_height as isize
            };
            session.scroll_preview(page);
            None
        }
        KeyCode::Backspace => {
            session.backspace();
            None
        }
        KeyCode::Char(ch) => {
            session.insert_char(ch);
            None
        }
        _ => None,
    }
}

fn handle_mouse(session: &mut ExploreSession, mouse: MouseEvent) {
    if let Some(help) = &mut session.help {
        match mouse.kind {
            MouseEventKind::ScrollUp => help.scroll(-3),
            MouseEventKind::ScrollDown => help.scroll(3),
            _ => {}
        }
        return;
    }
    match mouse.kind {
        MouseEventKind::ScrollUp => session.scroll_preview(-3),
        MouseEventKind::ScrollDown => session.scroll_preview(3),
        _ => {}
    }
}

fn filter_view(input: &str, cursor: usize, width: u16) -> (String, u16) {
    let width = width as usize;
    if width == 0 {
        return (String::new(), 0);
    }

    let prefix = Line::raw(&input[..cursor]);
    let graphemes = prefix
        .styled_graphemes(Style::default())
        .collect::<Vec<_>>();
    let mut start = cursor;
    let mut cursor_column = 0;
    for grapheme in graphemes.iter().rev() {
        let size = Span::raw(grapheme.symbol).width();
        if cursor_column + size > width - 1 {
            break;
        }
        start -= grapheme.symbol.len();
        cursor_column += size;
    }
    let source = Line::raw(&input[start..]);
    let mut visible = String::new();
    let mut columns = 0;
    for grapheme in source.styled_graphemes(Style::default()) {
        let size = Span::raw(grapheme.symbol).width();
        if columns + size > width {
            break;
        }
        visible.push_str(grapheme.symbol);
        columns += size;
    }
    (visible, cursor_column as u16)
}

fn help_layout(area: Rect) -> (Rect, Rect, Rect) {
    let width = area.width.saturating_sub(2).min(88);
    let height = area.height.saturating_sub(2).min(28);
    let popup = Rect::new(
        area.x + (area.width - width) / 2,
        area.y + (area.height - height) / 2,
        width,
        height,
    );
    let inner = Block::default().borders(Borders::ALL).inner(popup);
    let parts = Layout::vertical([Constraint::Min(0), Constraint::Length(2)]).split(inner);
    (popup, parts[0], parts[1])
}

// Pre-wrap the static help so its scroll bounds match the rendered lines without
// relying on ratatui's unstable rendered-line-info API.
fn help_lines(width: u16) -> Vec<Line<'static>> {
    let width = usize::from(width.max(1));
    let mut lines = Vec::new();
    for source in EXPLORER_HELP.lines() {
        let mut line = String::new();
        for word in source.split_whitespace() {
            if !line.is_empty() && line.chars().count() + 1 + word.chars().count() > width {
                lines.push(Line::raw(std::mem::take(&mut line)));
            }
            if !line.is_empty() {
                line.push(' ');
            }
            for ch in word.chars() {
                if line.chars().count() == width {
                    lines.push(Line::raw(std::mem::take(&mut line)));
                }
                line.push(ch);
            }
        }
        lines.push(Line::raw(line));
    }
    lines
}

fn update_viewport(session: &mut ExploreSession, area: Rect) {
    session.narrow_terminal = area.width < 80;
    let preview_area = preview_area_for(area, session.preview_fullscreen);
    let width = preview_area.width.saturating_sub(2);
    session.preview.viewport_height = preview_area.height.saturating_sub(2);
    if session.preview.viewport_width != width || session.preview.viewport_width == 0 {
        session.preview.visual_lines = preview_render_lines(session, width);
        session.preview.viewport_width = width;
    }
    session.clamp_preview_scroll();
    if session.stream.is_some() && session.preview.follow_tail {
        session.preview.preview_scroll = session.max_preview_scroll();
    }
    if let Some(help) = &mut session.help {
        let (_, content, _) = help_layout(area);
        help.max_scroll = help_lines(content.width)
            .len()
            .saturating_sub(content.height as usize)
            .min(u16::MAX as usize) as u16;
        help.scroll = help.scroll.min(help.max_scroll);
    }
}

fn render_help(frame: &mut Frame, help: &HelpState) {
    let (popup, content, footer) = help_layout(frame.area());
    frame.render_widget(Clear, popup);
    frame.render_widget(
        Block::default()
            .title("Explorer help")
            .borders(Borders::ALL),
        popup,
    );
    frame.render_widget(
        Paragraph::new(help_lines(content.width)).scroll((help.scroll, 0)),
        content,
    );
    frame.render_widget(
        Paragraph::new("Up/Down/PgUp/PgDn scroll\nF1/Esc close help"),
        footer,
    );
}

fn render(frame: &mut Frame, session: &ExploreSession) {
    let outer = explorer_layout(frame.area());
    let wide_preview = wide_preview(frame.area(), session.preview_fullscreen);

    let inputs = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(3), Constraint::Length(3)])
        .split(outer[0]);

    let body = if wide_preview {
        Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(100)])
            .split(outer[1])
    } else {
        Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(36), Constraint::Percentage(64)])
            .split(outer[1])
    };

    let filter_width = inputs[0].width.saturating_sub(2);
    let (filter_input, filter_cursor_column) = filter_view(
        &session.filter.text.input,
        session.filter.text.cursor,
        filter_width,
    );
    let filter_title = if session.active_input == ActiveInput::Filter {
        "Filter *"
    } else {
        "Filter"
    };
    let filter = Paragraph::new(filter_input)
        .block(Block::default().title(filter_title).borders(Borders::ALL));
    frame.render_widget(filter, inputs[0]);

    let output_width = inputs[1].width.saturating_sub(2);
    let (output_input, output_cursor_column) =
        filter_view(&session.output.input, session.output.cursor, output_width);
    let output_title = if session.active_input == ActiveInput::Output {
        "Output *"
    } else {
        "Output"
    };
    let output = Paragraph::new(output_input)
        .block(Block::default().title(output_title).borders(Borders::ALL));
    frame.render_widget(output, inputs[1]);

    let (cursor_area, cursor_column) = match session.active_input {
        ActiveInput::Filter => (inputs[0], filter_cursor_column),
        ActiveInput::Output => (inputs[1], output_cursor_column),
    };
    let cursor_x = cursor_area.x + 1 + cursor_column;
    if session.help.is_none() {
        frame.set_cursor_position(Position::new(cursor_x, cursor_area.y + 1));
    }

    let preview_area = if wide_preview {
        body[0]
    } else {
        let fields = session
            .data
            .fields
            .iter()
            .skip(session.preview.field_scroll)
            .take(body[0].height.saturating_sub(2) as usize)
            .map(|field| {
                let required = if field.optional {
                    "optional"
                } else {
                    "required"
                };
                let examples = if field.examples.is_empty() {
                    String::new()
                } else {
                    format!("  e.g. {}", field.examples.join(", "))
                };
                ListItem::new(Line::from(vec![
                    Span::styled(
                        field.path.clone(),
                        Style::default().add_modifier(Modifier::BOLD),
                    ),
                    Span::raw(format!(
                        "  {}  {required}  {}/{} docs{examples}",
                        field.types.join("|"),
                        field.documents,
                        session
                            .data
                            .values
                            .len()
                            .min(session.config.max_schema_documents),
                    )),
                ]))
            })
            .collect::<Vec<_>>();
        frame.render_widget(
            List::new(fields).block(
                Block::default()
                    .title(if session.data.fields.is_empty() {
                        "Fields 0/0".to_owned()
                    } else {
                        format!(
                            "Fields {}/{}",
                            session
                                .preview
                                .field_scroll
                                .saturating_add(1)
                                .min(session.data.fields.len()),
                            session.data.fields.len()
                        )
                    })
                    .borders(Borders::ALL),
            ),
            body[0],
        );
        body[1]
    };

    let preview_width = preview_area.width.saturating_sub(2);
    let preview_lines = if session.preview.viewport_width == preview_width && preview_width > 0 {
        session
            .preview
            .visual_lines
            .iter()
            .skip(session.preview.preview_scroll)
            .take(preview_area.height.saturating_sub(2) as usize)
            .cloned()
            .collect::<Vec<_>>()
    } else {
        preview_render_lines(session, preview_width)
            .into_iter()
            .skip(session.preview.preview_scroll)
            .take(preview_area.height.saturating_sub(2) as usize)
            .collect()
    };
    frame.render_widget(
        Paragraph::new(preview_lines)
            .block(Block::default().title("Preview").borders(Borders::ALL)),
        preview_area,
    );

    let status = if frame.area().width < 80 {
        if let Some(error) = &session.filter.error {
            format!("Error: {error}")
        } else if let Some(message) = &session.message {
            message.clone()
        } else {
            session.preview_status()
        }
    } else {
        session.status_line()
    };
    let stream_status = session.stream.as_ref().map(|stream| {
        let status = if stream.ended { "complete" } else { "live" };
        format!(
            "stream {status}: {} docs, {} errors",
            session.data.values.len(),
            stream.errors.len()
        )
    });
    let help_primary = "F1 help | Tab complete | Ctrl-O input | Ctrl-S save | Ctrl-Y jq";
    let help_scroll = session
        .preview
        .cache
        .first_error
        .as_ref()
        .map(|error| {
            format!(
                "skipped incompatible records: {}",
                output::escape_terminal(error)
            )
        })
        .unwrap_or_else(|| {
            "Ctrl-T tail | PgUp/PgDn/Wheel scroll | Enter print | Esc quit".to_owned()
        });
    let help_primary = if frame.area().width < 15 {
        "F1 | Esc"
    } else if frame.area().width < 24 {
        "F1 help | Esc"
    } else if frame.area().width < 34 {
        "F1 help | Enter | Esc"
    } else if frame.area().width < 38 {
        "F1 help | ^O input | Enter | Esc"
    } else if frame.area().width < 80 {
        "F1 help | Ctrl-O input | Enter | Esc"
    } else {
        help_primary
    };
    let hint = if let Some(stream_status) = stream_status {
        if frame.area().width < 80 {
            format!("{status}\n{stream_status}\n{help_primary}")
        } else {
            format!("{status}\n{stream_status}\n{help_primary}\n{help_scroll}")
        }
    } else if frame.area().width < 80 {
        format!("{status}\n{help_primary}")
    } else {
        format!("{status}\n{help_primary}\n{help_scroll}")
    };
    let hint_style = if session.filter.error.is_some() {
        Style::default().fg(Color::Red)
    } else {
        Style::default()
    };
    frame.render_widget(
        Paragraph::new(hint)
            .style(hint_style)
            .block(Block::default().borders(Borders::ALL)),
        outer[2],
    );
    if let Some(help) = &session.help {
        render_help(frame, help);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stream_batches_are_bounded_and_preserve_end_and_errors() {
        let mut session = help_test_session();
        session.data.values.clear();
        session.stream = Some(StreamState {
            ended: false,
            errors: Vec::new(),
        });
        let (sender, receiver) = mpsc::channel();
        for _ in 0..STREAM_BATCH_SIZE + 1 {
            sender
                .send(StreamEvent::Line(br#"{"name":"batch"}"#.to_vec()))
                .unwrap();
        }
        sender
            .send(StreamEvent::Error("producer failure".to_owned()))
            .unwrap();
        sender.send(StreamEvent::End).unwrap();
        drain_stream(&mut session, &receiver);
        assert_eq!(session.data.values.len(), STREAM_BATCH_SIZE);
        assert!(!session.stream.as_ref().unwrap().ended);
        assert_eq!(preview_text(&session)[0], "batch");
        drain_stream(&mut session, &receiver);
        assert_eq!(session.data.values.len(), STREAM_BATCH_SIZE + 1);
        let stream = session.stream.as_ref().unwrap();
        assert!(stream.ended);
        assert_eq!(stream.errors, ["producer failure"]);
    }

    #[test]
    fn cached_preview_recovers_after_invalid_and_changed_filters() {
        let mut session = help_test_session();
        session.filter.text.input = ".name".to_owned();
        session.refresh();
        assert_eq!(session.compiled_filter.as_ref().unwrap().0, ".name");
        let initial = preview_text(&session);
        session.refresh();
        assert_eq!(preview_text(&session), initial);
        session.filter.text.input = ".[".to_owned();
        session.refresh();
        assert!(session.filter.error.is_some());
        assert!(session.preview.lines.is_empty());
        session.filter.text.input = ".name".to_owned();
        session.refresh();
        assert!(session.filter.error.is_none());
        assert_eq!(preview_text(&session), initial);
        session.output.input = "length".to_owned();
        session.refresh();
        assert_ne!(session.compiled_filter.as_ref().unwrap().0, ".name");
    }

    #[test]
    fn shorthand_path_filter_combines_with_any_output() {
        let document = parse_values(
            InputKind::Json,
            br#"{"log":{"message":"hello","level":"ERROR"}}"#,
            false,
        )
        .unwrap();
        for filter in ["log", ".log"] {
            for output in ["message", ".message", "{message}", "[.message]"] {
                let code = compose_explore_jq(filter, output).unwrap();
                let expected = compose_explore_jq(".log", output).unwrap();
                assert_eq!(
                    Matcher::compile(&code)
                        .unwrap()
                        .apply(document[0].clone())
                        .unwrap(),
                    Matcher::compile(&expected)
                        .unwrap()
                        .apply(document[0].clone())
                        .unwrap()
                );
            }
        }
    }

    #[test]
    fn stream_tail_counts_all_matches_pauses_and_exports_all_records() {
        let mut session = ExploreSession::new_streaming(
            "index>=0".to_owned(),
            "index".to_owned(),
            ExploreConfig {
                max_input_bytes: 100000,
                max_schema_documents: 10,
                max_preview_results: 3,
                max_schema_depth: 4,
            },
            output::OutputOptions::plain(),
        );
        for index in 0..20 {
            session.append_stream_values(vec![Val::obj(
                [(Val::from("index".to_owned()), Val::from(index as isize))]
                    .into_iter()
                    .collect(),
            )]);
        }
        assert_eq!(session.preview.match_count, 20);
        assert_eq!(preview_text(&session), ["17", "18", "19"]);
        session.toggle_follow_tail();
        session.append_stream_values(
            parse_values(InputKind::Json, br#"{"index":20}"#, false).unwrap(),
        );
        assert_eq!(preview_text(&session), ["17", "18", "19"]);
        assert_eq!(session.preview.match_count, 21);
        session.toggle_follow_tail();
        assert_eq!(preview_text(&session), ["18", "19", "20"]);
        let mut output = Vec::new();
        let mut errors = Vec::new();
        assert_eq!(write_explore_results(&session, &mut output, &mut errors), 0);
        assert_eq!(
            String::from_utf8(output).unwrap(),
            (0..21).map(|i| format!("{i}\n")).collect::<String>()
        );
        assert!(errors.is_empty());
    }

    #[test]
    fn incompatible_records_keep_valid_preview_and_export_with_error_status() {
        let mut session = help_test_session();
        session.data.values = parse_values(
            InputKind::Json,
            b"{\"message\":\"HELLO\"}\n{\"message\":[]}\n{\"message\":\"WORLD\"}",
            false,
        )
        .unwrap();
        session.filter.text.input = ".".to_owned();
        session.output.input = ".message | ascii_downcase".to_owned();
        session.refresh();
        assert!(session.filter.error.is_none());
        assert_eq!(session.preview.cache.error_count, 1);
        assert_eq!(preview_text(&session), ["hello", "world"]);
        let mut output = Vec::new();
        let mut errors = Vec::new();
        assert_eq!(write_explore_results(&session, &mut output, &mut errors), 2);
        assert_eq!(output, b"hello\nworld\n");
        assert!(String::from_utf8(errors).unwrap().contains("1 eval errors"));
        let mut snapshot = Vec::new();
        print_snapshot(&session, &mut snapshot).unwrap();
        let snapshot = String::from_utf8(snapshot).unwrap();
        assert!(snapshot.contains("preview:\n  hello\n  world"));
        assert!(snapshot.contains("skipped incompatible records"));
    }

    #[test]
    fn syntax_error_blocks_save_print_and_generated_filter_export() {
        let mut session = help_test_session();
        session.filter.text.input = ".[".to_owned();
        session.refresh();
        assert!(session.filter.error.is_some());
        assert_eq!(
            handle_key(&mut session, KeyCode::Enter, KeyModifiers::NONE),
            None
        );
        assert_eq!(
            handle_key(&mut session, KeyCode::Char('y'), KeyModifiers::CONTROL),
            None
        );
        session.save_filter();
        assert_eq!(session.message.as_deref(), Some("no valid filter to save"));
    }

    #[test]
    fn deferred_edit_batches_flush_before_print_and_jq_export() {
        for (key, modifiers, expected) in [
            (
                KeyCode::Enter,
                KeyModifiers::NONE,
                ExplorerExit::PrintResults,
            ),
            (
                KeyCode::Char('y'),
                KeyModifiers::CONTROL,
                ExplorerExit::PrintFilter,
            ),
        ] {
            let mut session = help_test_session();
            session.filter.text.input.clear();
            session.filter.text.cursor = 0;
            session.output.input = "name".to_owned();
            session.data.values =
                parse_values(InputKind::Json, br#"{"name":"Alice","age":23}"#, false).unwrap();
            session.refresh();
            session.defer_refresh = true;
            for ch in "age>=18".chars() {
                session.insert_char(ch);
            }
            assert!(session.refresh_pending);
            assert_eq!(handle_key(&mut session, key, modifiers), Some(expected));
            assert!(!session.refresh_pending);
            assert_eq!(session.filter.generated, "select(.age >= 18) | .name");
            assert_eq!(preview_text(&session), ["Alice"]);
        }
    }

    #[test]
    fn zero_preview_limit_disables_display_without_losing_match_count_or_export() {
        let mut session = help_test_session();
        session.preview.limit = 0;
        session.refresh();
        assert_eq!(session.preview.match_count, 1);
        assert!(session.preview.lines.is_empty());
        assert!(preview_values(
            &session.data.values,
            ".name",
            0,
            &output::OutputOptions::plain()
        )
        .unwrap()
        .is_empty());
        let mut output = Vec::new();
        let mut errors = Vec::new();
        assert_eq!(write_explore_results(&session, &mut output, &mut errors), 0);
        assert_eq!(output, b"Alice\n");
    }

    #[test]
    fn seeded_unicode_editor_operations_preserve_cursor_and_recover() {
        let mut seed = 0x54454954_u64;
        let characters = ['a', '.', '[', '"', 'ä', '界', '🦀', '=', '\\'];
        let mut session = help_test_session();
        for _ in 0..500 {
            seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
            match (seed >> 32) % 9 {
                0 => session.backspace(),
                1 => session.delete(),
                2 => session.move_cursor_left(),
                3 => session.move_cursor_right(),
                4 => session.toggle_active_input(),
                5 => {
                    session.active_text_mut().cursor = 0;
                }
                6 => {
                    session.active_text_mut().cursor = session.active_text().input.len();
                }
                _ => session.insert_char(characters[(seed as usize) % characters.len()]),
            }
            for text in [&session.filter.text, &session.output] {
                assert!(text.cursor <= text.input.len());
                assert!(text.input.is_char_boundary(text.cursor));
            }
        }
        session.filter.text.input = "name".to_owned();
        session.output.input.clear();
        session.refresh();
        assert!(session.filter.error.is_none());
        assert_eq!(preview_text(&session), ["Alice"]);
    }

    #[test]
    fn seeded_filter_output_and_stream_partitions_match_independent_reference() {
        let mut seed = 0x4a67726570_u64;
        for case in 0..160 {
            let mut random = || {
                seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
                seed >> 32
            };
            let count = (random() % 80 + 1) as usize;
            let threshold = (random() % 100) as usize;
            let limit = (random() % 12 + 1) as usize;
            let mut input = String::new();
            for index in 0..count {
                input.push_str(&format!(
                    "{{\"index\":{index},\"score\":{},\"message\":\"m{index}\"}}\n",
                    random() % 100
                ));
            }
            let values = parse_values(InputKind::Json, input.as_bytes(), false).unwrap();
            let filters = [
                format!("score>={threshold}"),
                format!(".score>={threshold}"),
                format!("select(.score >= {threshold})"),
            ];
            let outputs = [
                "message",
                ".message",
                "{message,index}",
                "[.index,.message]",
            ];
            let output = outputs[case % outputs.len()];
            let reference = format!(
                "select(.score >= {threshold}) | {}",
                shortcuts::output_expression_to_jq(output).unwrap()
            );
            let expected = preview_values(
                &values,
                &reference,
                usize::MAX,
                &output::OutputOptions::plain(),
            )
            .unwrap();
            let config = ExploreConfig {
                max_input_bytes: 100000,
                max_schema_documents: 10,
                max_preview_results: limit,
                max_schema_depth: 4,
            };
            let mut session = ExploreSession::new_streaming(
                filters[case % filters.len()].clone(),
                output.to_owned(),
                config,
                output::OutputOptions::plain(),
            );
            let chunk = (random() % 17 + 1) as usize;
            for batch in values.chunks(chunk) {
                session.append_stream_values(batch.to_vec());
            }
            assert_eq!(
                session.preview.match_count,
                expected.len(),
                "case={case}, seed={seed}"
            );
            let mut actual = Vec::new();
            let mut errors = Vec::new();
            assert_eq!(
                write_explore_results(&session, &mut actual, &mut errors),
                if expected.is_empty() { 1 } else { 0 },
                "case={case}"
            );
            assert_eq!(
                String::from_utf8(actual).unwrap(),
                expected
                    .iter()
                    .map(|line| format!("{line}\n"))
                    .collect::<String>(),
                "case={case}"
            );
            assert_eq!(
                preview_text(&session),
                expected[expected.len().saturating_sub(limit)..],
                "case={case}"
            );
            session.filter.text.input = ".[".to_owned();
            session.refresh();
            assert!(session.filter.error.is_some());
            session.filter.text.input = filters[case % filters.len()].clone();
            session.refresh();
            assert_eq!(session.preview.match_count, expected.len());
        }
    }
    use ratatui::backend::TestBackend;

    fn preview_text(session: &ExploreSession) -> Vec<String> {
        session
            .preview
            .lines
            .iter()
            .map(|line| line.text.clone())
            .collect()
    }

    fn help_test_session() -> ExploreSession {
        ExploreSession::new(
            parse_values(InputKind::Json, br#"{"name":"Alice"}"#, false).unwrap(),
            "name".to_owned(),
            String::new(),
            ExploreConfig {
                max_input_bytes: 1024,
                max_schema_documents: 10,
                max_preview_results: 10,
                max_schema_depth: 4,
            },
            output::OutputOptions::plain(),
        )
    }

    fn rendered_session(session: &ExploreSession, width: u16, height: u16) -> String {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        terminal.draw(|frame| render(frame, session)).unwrap();
        terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect()
    }

    #[test]
    fn help_captures_actions_and_escape_returns_to_unchanged_editor() {
        let mut session = help_test_session();
        handle_key(&mut session, KeyCode::F(1), KeyModifiers::NONE);
        assert!(session.help.is_some());
        for (key, modifiers) in [
            (KeyCode::Char('?'), KeyModifiers::NONE),
            (KeyCode::Enter, KeyModifiers::NONE),
            (KeyCode::Tab, KeyModifiers::NONE),
            (KeyCode::Char('s'), KeyModifiers::CONTROL),
            (KeyCode::Char('y'), KeyModifiers::CONTROL),
            (KeyCode::Char('o'), KeyModifiers::CONTROL),
        ] {
            assert!(handle_key(&mut session, key, modifiers).is_none());
        }
        assert_eq!(session.filter.text.input, "name");
        assert_eq!(session.active_input, ActiveInput::Filter);
        assert_eq!(preview_text(&session), ["Alice"]);
        assert!(session.message.is_none());
        assert!(handle_key(&mut session, KeyCode::Esc, KeyModifiers::NONE).is_none());
        assert!(session.help.is_none());
        assert!(matches!(
            handle_key(&mut session, KeyCode::Esc, KeyModifiers::NONE),
            Some(ExplorerExit::Quit)
        ));
    }

    #[test]
    fn f1_toggles_help_and_question_mark_remains_editable_text() {
        let mut session = help_test_session();
        handle_key(&mut session, KeyCode::F(1), KeyModifiers::NONE);
        handle_key(&mut session, KeyCode::F(1), KeyModifiers::NONE);
        assert!(session.help.is_none());
        handle_key(&mut session, KeyCode::Char('?'), KeyModifiers::NONE);
        assert_eq!(session.filter.text.input, "name?");
        handle_key(&mut session, KeyCode::F(1), KeyModifiers::NONE);
        assert!(matches!(
            handle_key(&mut session, KeyCode::Char('c'), KeyModifiers::CONTROL),
            Some(ExplorerExit::Quit)
        ));
    }

    #[test]
    fn help_scroll_is_bounded_and_clamped_after_resize() {
        let mut session = help_test_session();
        handle_key(&mut session, KeyCode::F(1), KeyModifiers::NONE);
        update_viewport(&mut session, Rect::new(0, 0, 40, 20));
        let narrow_max = session.help.as_ref().unwrap().max_scroll;
        assert!(narrow_max > 0);
        for _ in 0..10 {
            handle_key(&mut session, KeyCode::PageDown, KeyModifiers::NONE);
        }
        assert_eq!(session.help.as_ref().unwrap().scroll, narrow_max);
        update_viewport(&mut session, Rect::new(0, 0, 120, 40));
        let help = session.help.as_ref().unwrap();
        assert!(help.max_scroll < narrow_max);
        assert_eq!(help.scroll, help.max_scroll);
        handle_key(&mut session, KeyCode::Home, KeyModifiers::NONE);
        handle_key(&mut session, KeyCode::PageUp, KeyModifiers::NONE);
        assert_eq!(session.help.as_ref().unwrap().scroll, 0);
        handle_key(&mut session, KeyCode::End, KeyModifiers::NONE);
        assert_eq!(
            session.help.as_ref().unwrap().scroll,
            session.help.as_ref().unwrap().max_scroll
        );
    }

    #[test]
    fn narrow_layout_preserves_preview_and_essential_actions() {
        let session = help_test_session();
        for height in [16, 20, 30] {
            let rendered = rendered_session(&session, 40, height);
            assert!(rendered.contains("Alice"));
            assert!(rendered.contains("Preview"));
            assert!(rendered.contains("1 matches"));
            assert!(rendered.contains("F1 help | Ctrl-O input | Enter | Esc"));
            assert!(!rendered.contains("Fields"));
            assert_eq!(
                preview_area_for(Rect::new(0, 0, 40, height), false).width,
                40
            );
        }
        for width in [80, 120] {
            let rendered = rendered_session(&session, width, 24);
            assert!(rendered.contains("Fields"));
            assert!(rendered.contains("Tab complete"));
            assert!(rendered.contains("Ctrl-S save"));
        }
    }

    #[test]
    fn help_keeps_close_action_visible_on_small_terminals() {
        let mut session = help_test_session();
        handle_key(&mut session, KeyCode::F(1), KeyModifiers::NONE);
        for (width, height) in [(40, 16), (40, 20), (80, 24), (120, 30)] {
            update_viewport(&mut session, Rect::new(0, 0, width, height));
            let rendered = rendered_session(&session, width, height);
            assert!(rendered.contains("Explorer help"));
            assert!(rendered.contains("Filter selects input records."));
            assert!(rendered.contains("F1/Esc close help"));
        }
        for width in [1, 10, 36, 86] {
            assert!(help_lines(width)
                .iter()
                .all(|line| line.width() <= width as usize));
        }
    }

    #[test]
    fn narrow_filter_arrows_scroll_visible_preview_not_hidden_fields() {
        let mut session = help_test_session();
        update_viewport(&mut session, Rect::new(0, 0, 40, 20));
        session.preview.lines.extend((0..20).map(|i| PreviewLine {
            text: i.to_string(),
            style: Style::default(),
            badge: None,
        }));
        session.preview.viewport_width = 0;
        update_viewport(&mut session, Rect::new(0, 0, 40, 20));
        handle_key(&mut session, KeyCode::Down, KeyModifiers::NONE);
        assert_eq!(session.preview.preview_scroll, 1);
        assert_eq!(session.preview.field_scroll, 0);
        handle_key(&mut session, KeyCode::Up, KeyModifiers::NONE);
        assert_eq!(session.preview.preview_scroll, 0);
    }

    #[test]
    fn preview_applies_shortcut_filter() {
        let values = parse_values(
            InputKind::Json,
            br#"{"name":"Alice","status":"active"}
{"name":"Bob","status":"blocked"}
"#,
            false,
        )
        .unwrap();
        let jq = shortcuts::expression_to_jq("status=active").unwrap();

        let preview = preview_values(&values, &jq, 10, &output::OutputOptions::plain()).unwrap();

        assert_eq!(preview, [r#"{"name":"Alice","status":"active"}"#]);
    }

    #[cfg(feature = "yaml")]
    #[test]
    fn unknown_extension_can_fall_back_to_yaml() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("selection");
        std::fs::write(&file, "name: Alice\n").unwrap();

        let values = read_values(Some(&file), 1024 * 1024).unwrap();
        let jq = shortcuts::expression_to_jq("name").unwrap();
        let preview = preview_values(&values, &jq, 10, &output::OutputOptions::plain()).unwrap();

        assert_eq!(preview, ["Alice"]);
    }

    #[test]
    fn refuses_input_larger_than_limit() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("large.json");
        std::fs::write(&file, "{\"name\":\"Alice\"}\n").unwrap();

        let error = read_values(Some(&file), 4).unwrap_err();

        assert!(error.contains("--max-input-bytes"));
    }

    #[test]
    fn combines_filter_and_output_projection() {
        assert_eq!(
            compose_explore_jq("log.level=ERROR", "message").unwrap(),
            "select((if type == \"object\" and has(\"log.level\") then .[\"log.level\"] else .log.level end) == \"ERROR\") | .message"
        );
        assert_eq!(compose_explore_jq("", "message").unwrap(), ".message");
        assert_eq!(
            compose_explore_jq("status=active", "").unwrap(),
            "select(.status == \"active\") | ."
        );
        assert_eq!(
            compose_explore_jq("log.level=ERROR", "{message, level: .log.level}").unwrap(),
            "select((if type == \"object\" and has(\"log.level\") then .[\"log.level\"] else .log.level end) == \"ERROR\") | {message, level: .log.level}"
        );
    }

    #[test]
    fn output_input_can_be_edited_live() {
        let values = parse_values(
            InputKind::Json,
            br#"{"name":"Alice","status":"active"}"#,
            false,
        )
        .unwrap();
        let config = ExploreConfig {
            max_input_bytes: 1024 * 1024,
            max_schema_documents: 10,
            max_preview_results: 10,
            max_schema_depth: 4,
        };
        let mut session = ExploreSession::new(
            values,
            "status=active".to_owned(),
            String::new(),
            config,
            output::OutputOptions::plain(),
        );

        handle_key(&mut session, KeyCode::Char('o'), KeyModifiers::CONTROL);
        for ch in "name".chars() {
            handle_key(&mut session, KeyCode::Char(ch), KeyModifiers::NONE);
        }

        assert_eq!(session.active_input, ActiveInput::Output);
        assert_eq!(session.output.input, "name");
        assert_eq!(
            session.filter.generated,
            "select(.status == \"active\") | .name"
        );
        assert_eq!(preview_text(&session), ["Alice"]);
    }

    #[test]
    fn output_options_can_be_toggled_live() {
        let config = ExploreConfig {
            max_input_bytes: 1024 * 1024,
            max_schema_documents: 10,
            max_preview_results: 10,
            max_schema_depth: 4,
        };
        let mut session = ExploreSession::new(
            Vec::new(),
            String::new(),
            String::new(),
            config,
            output::OutputOptions::plain(),
        );

        handle_key(&mut session, KeyCode::Char('f'), KeyModifiers::CONTROL);
        handle_key(&mut session, KeyCode::Char('l'), KeyModifiers::CONTROL);
        handle_key(&mut session, KeyCode::Char('k'), KeyModifiers::CONTROL);

        assert!(session.output_options.pretty);
        assert!(session.output_options.color_level);
        assert!(session.output_options.no_color);
    }

    #[test]
    fn output_options_apply_to_live_preview() {
        let values = parse_values(
            InputKind::Json,
            br#"{"log":{"level":"ERROR"},"message":"boom","details":{"code":42}}"#,
            false,
        )
        .unwrap();
        let config = ExploreConfig {
            max_input_bytes: 1024 * 1024,
            max_schema_documents: 10,
            max_preview_results: 10,
            max_schema_depth: 4,
        };
        let mut session = ExploreSession::new(
            values,
            String::new(),
            "{message,details}".to_owned(),
            config,
            output::OutputOptions::plain(),
        );

        handle_key(&mut session, KeyCode::Char('f'), KeyModifiers::CONTROL);
        handle_key(&mut session, KeyCode::Char('l'), KeyModifiers::CONTROL);

        let text = preview_text(&session);
        assert!(text.iter().any(|line| line == "{"));
        assert!(text.iter().any(|line| line.contains("\"message\"")));
        assert!(session
            .preview
            .lines
            .iter()
            .any(|line| line.style.fg == Some(Color::Rgb(255, 80, 90))
                && line.badge.is_some_and(|badge| badge.label == "ERROR")));
    }

    #[test]
    fn render_level_color_uses_visible_line_foreground() {
        let values = parse_values(
            InputKind::Json,
            br#"{"log":{"level":"ERROR"},"message":"boom"}"#,
            false,
        )
        .unwrap();
        let config = ExploreConfig {
            max_input_bytes: 1024 * 1024,
            max_schema_documents: 10,
            max_preview_results: 10,
            max_schema_depth: 4,
        };
        let mut session = ExploreSession::new(
            values,
            String::new(),
            "message".to_owned(),
            config,
            output::OutputOptions {
                pretty: false,
                json_color: false,
                color_level: true,
                no_color: false,
                color_level_field: None,
            },
        );
        session.preview_fullscreen = true;
        let backend = TestBackend::new(100, 24);
        let mut terminal = Terminal::new(backend).unwrap();

        terminal.draw(|frame| render(frame, &session)).unwrap();
        let buffer = terminal.backend().buffer();

        assert!(buffer
            .content()
            .iter()
            .any(|cell| cell.symbol() == "b" && cell.fg == Color::Rgb(255, 80, 90)));
        assert!(buffer
            .content()
            .iter()
            .any(|cell| cell.symbol() == "E" && cell.fg == Color::Rgb(255, 80, 90)));
    }

    #[test]
    fn output_mode_arrows_scroll_preview() {
        let values = parse_values(
            InputKind::Json,
            br#"{"items":[{"name":"a"},{"name":"b"},{"name":"c"}]}"#,
            false,
        )
        .unwrap();
        let config = ExploreConfig {
            max_input_bytes: 1024 * 1024,
            max_schema_documents: 10,
            max_preview_results: 10,
            max_schema_depth: 4,
        };
        let mut session = ExploreSession::new(
            values,
            String::new(),
            ".".to_owned(),
            config,
            output::OutputOptions::plain(),
        );
        handle_key(&mut session, KeyCode::Char('f'), KeyModifiers::CONTROL);
        handle_key(&mut session, KeyCode::Char('o'), KeyModifiers::CONTROL);

        handle_key(&mut session, KeyCode::Down, KeyModifiers::NONE);

        assert_eq!(session.preview.preview_scroll, 1);
    }

    #[test]
    fn fullscreen_logs_arrows_scroll_preview_even_from_filter() {
        let values = parse_values(
            InputKind::Json,
            br#"{"items":[{"name":"a"},{"name":"b"},{"name":"c"}]}"#,
            false,
        )
        .unwrap();
        let config = ExploreConfig {
            max_input_bytes: 1024 * 1024,
            max_schema_documents: 10,
            max_preview_results: 10,
            max_schema_depth: 4,
        };
        let mut session = ExploreSession::new(
            values,
            String::new(),
            ".".to_owned(),
            config,
            output::OutputOptions::plain(),
        );
        handle_key(&mut session, KeyCode::Char('f'), KeyModifiers::CONTROL);
        handle_key(&mut session, KeyCode::Char('g'), KeyModifiers::CONTROL);

        handle_key(&mut session, KeyCode::Down, KeyModifiers::NONE);

        assert_eq!(session.active_input, ActiveInput::Filter);
        assert_eq!(session.preview.preview_scroll, 1);
    }

    #[test]
    fn mouse_wheel_scrolls_preview() {
        let values = parse_values(
            InputKind::Json,
            br#"{"items":[{"name":"a"},{"name":"b"},{"name":"c"}]}"#,
            false,
        )
        .unwrap();
        let config = ExploreConfig {
            max_input_bytes: 1024 * 1024,
            max_schema_documents: 10,
            max_preview_results: 10,
            max_schema_depth: 4,
        };
        let mut session = ExploreSession::new(
            values,
            String::new(),
            ".".to_owned(),
            config,
            output::OutputOptions::plain(),
        );
        handle_key(&mut session, KeyCode::Char('f'), KeyModifiers::CONTROL);

        handle_mouse(
            &mut session,
            MouseEvent {
                kind: MouseEventKind::ScrollDown,
                column: 10,
                row: 10,
                modifiers: KeyModifiers::NONE,
            },
        );

        assert_eq!(session.preview.preview_scroll, 3);
    }

    #[test]
    fn toggles_preview_fullscreen() {
        let config = ExploreConfig {
            max_input_bytes: 1024 * 1024,
            max_schema_documents: 10,
            max_preview_results: 10,
            max_schema_depth: 4,
        };
        let mut session = ExploreSession::new(
            Vec::new(),
            String::new(),
            String::new(),
            config,
            output::OutputOptions::plain(),
        );

        handle_key(&mut session, KeyCode::Char('g'), KeyModifiers::CONTROL);

        assert!(session.preview_fullscreen);
        assert!(session.status_line().contains("logs:full"));
    }

    #[test]
    fn autocomplete_cycles_candidates_with_type_info() {
        let values = parse_values(
            InputKind::Json,
            br#"{"name":"Alice","namespace":"prod","node":"worker-1"}"#,
            false,
        )
        .unwrap();
        let config = ExploreConfig {
            max_input_bytes: 1024 * 1024,
            max_schema_documents: 10,
            max_preview_results: 10,
            max_schema_depth: 4,
        };
        let mut session = ExploreSession::new(
            values,
            "n".to_owned(),
            String::new(),
            config,
            output::OutputOptions::plain(),
        );

        handle_key(&mut session, KeyCode::Tab, KeyModifiers::NONE);

        assert_eq!(session.filter.text.input, ".name");
        assert!(session
            .message
            .as_deref()
            .unwrap_or("")
            .contains("[string]"));

        handle_key(&mut session, KeyCode::Tab, KeyModifiers::NONE);

        assert_eq!(session.filter.text.input, ".namespace");
        assert!(session
            .message
            .as_deref()
            .unwrap_or("")
            .contains("completion 2/3"));
    }

    #[test]
    fn completion_treats_hostile_keys_as_data_in_both_editors() {
        for key in [
            "select(true) | 42",
            "{injected:42}",
            "[42]",
            "\"a\" + \"b\"",
            "a=b",
            "a.b",
            "true",
            "1",
            "back\\slash",
            "line\nfeed",
        ] {
            let document = Val::obj(
                [(Val::from(key.to_owned()), Val::from("expected".to_owned()))]
                    .into_iter()
                    .collect(),
            );
            for active in [ActiveInput::Filter, ActiveInput::Output] {
                let config = ExploreConfig {
                    max_input_bytes: 1024,
                    max_schema_documents: 10,
                    max_preview_results: 10,
                    max_schema_depth: 4,
                };
                let mut session = ExploreSession::new(
                    vec![document.clone()],
                    String::new(),
                    String::new(),
                    config,
                    output::OutputOptions::plain(),
                );
                session.active_input = active;
                session.insert_char(key.chars().next().unwrap());
                session.autocomplete_filter();
                assert!(
                    session.filter.error.is_none(),
                    "{key}: {:?}",
                    session.filter.error
                );
                assert_eq!(preview_text(&session), ["expected"], "{key}");
                // Editing a completed expression must keep its quoted-key boundary.
                session.insert_char(' ');
                assert_eq!(preview_text(&session), ["expected"], "{key}");
            }
        }
    }

    #[test]
    fn terminal_display_encodes_controls_but_raw_export_preserves_them() {
        let values = parse_values(InputKind::Json, br#"{"level":"ERROR","message":"before\u001b]52;c;payload\u0007\r\t\u009bafter","evil\u001b[2J":"example\u007f"}"#, false).unwrap();
        let config = ExploreConfig {
            max_input_bytes: 1024,
            max_schema_documents: 10,
            max_preview_results: 10,
            max_schema_depth: 4,
        };
        let mut options = output::OutputOptions::plain();
        options.color_level = true;
        options.no_color = false;
        let session = ExploreSession::new(
            values.clone(),
            String::new(),
            "message".to_owned(),
            config,
            options.clone(),
        );
        assert!(session.preview.lines[0].badge.is_some());
        assert!(!session.preview.lines[0].text.chars().any(char::is_control));
        assert!(session.preview.lines[0].text.contains("\\u{1b}]52"));
        let mut snapshot = Vec::new();
        print_snapshot(&session, &mut snapshot).unwrap();
        let snapshot = String::from_utf8(snapshot).unwrap();
        assert!(!snapshot.contains("\u{1b}]52"));
        assert!(!snapshot.contains("\u{1b}[2J"));
        assert_eq!(
            snapshot.contains("\u{1b}[31m"),
            std::env::var_os("NO_COLOR").is_none()
        );
        let raw = preview_values(&values, ".message", 10, &output::OutputOptions::plain()).unwrap();
        assert!(raw[0].contains("\u{1b}]52"));
    }

    #[test]
    fn cursor_editing_inserts_and_deletes_in_middle() {
        let values = parse_values(InputKind::Json, br#"{"name":"Alice"}"#, false).unwrap();
        let config = ExploreConfig {
            max_input_bytes: 1024 * 1024,
            max_schema_documents: 10,
            max_preview_results: 10,
            max_schema_depth: 4,
        };
        let mut session = ExploreSession::new(
            values,
            "name".to_owned(),
            String::new(),
            config,
            output::OutputOptions::plain(),
        );

        handle_key(&mut session, KeyCode::Left, KeyModifiers::NONE);
        handle_key(&mut session, KeyCode::Left, KeyModifiers::NONE);
        handle_key(&mut session, KeyCode::Char('X'), KeyModifiers::NONE);

        assert_eq!(session.filter.text.input, "naXme");
        assert_eq!(session.filter.text.cursor, 3);

        handle_key(&mut session, KeyCode::Backspace, KeyModifiers::NONE);
        handle_key(&mut session, KeyCode::Delete, KeyModifiers::NONE);

        assert_eq!(session.filter.text.input, "nae");
        assert_eq!(session.filter.text.cursor, 2);
    }

    #[test]
    fn history_navigation_loads_previous_filters() {
        let values = parse_values(
            InputKind::Json,
            br#"{"name":"Alice","status":"active"}"#,
            false,
        )
        .unwrap();
        let config = ExploreConfig {
            max_input_bytes: 1024 * 1024,
            max_schema_documents: 10,
            max_preview_results: 10,
            max_schema_depth: 4,
        };
        let mut session = ExploreSession::new(
            values,
            String::new(),
            String::new(),
            config,
            output::OutputOptions::plain(),
        );
        session.filter.history = vec!["name".to_owned(), "status=active".to_owned()];

        handle_key(&mut session, KeyCode::Char('p'), KeyModifiers::CONTROL);

        assert_eq!(session.filter.text.input, "status=active");
        assert_eq!(
            preview_text(&session),
            [r#"{"name":"Alice","status":"active"}"#]
        );

        handle_key(&mut session, KeyCode::Char('p'), KeyModifiers::CONTROL);

        assert_eq!(session.filter.text.input, "name");
        assert_eq!(preview_text(&session), ["Alice"]);
    }

    #[test]
    fn render_snapshot_contains_core_regions() {
        let values = parse_values(
            InputKind::Json,
            br#"{"name":"Alice","status":"active"}"#,
            false,
        )
        .unwrap();
        let config = ExploreConfig {
            max_input_bytes: 1024 * 1024,
            max_schema_documents: 10,
            max_preview_results: 10,
            max_schema_depth: 4,
        };
        let session = ExploreSession::new(
            values,
            "name".to_owned(),
            String::new(),
            config,
            output::OutputOptions::plain(),
        );
        let backend = TestBackend::new(100, 24);
        let mut terminal = Terminal::new(backend).unwrap();

        terminal.draw(|frame| render(frame, &session)).unwrap();
        let rendered = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>();

        assert!(rendered.contains("Filter"));
        assert!(rendered.contains("Fields 1/2"));
        assert!(rendered.contains("Preview"));
        assert!(rendered.contains("Alice"));
        assert!(rendered.contains("PgUp/PgDn/Wheel scroll"));
    }

    #[test]
    fn render_stream_status_keeps_shortcut_help_visible() {
        let config = ExploreConfig {
            max_input_bytes: 1024 * 1024,
            max_schema_documents: 10,
            max_preview_results: 10,
            max_schema_depth: 4,
        };
        let session = ExploreSession::new_streaming(
            String::new(),
            String::new(),
            config,
            output::OutputOptions::plain(),
        );
        let backend = TestBackend::new(100, 24);
        let mut terminal = Terminal::new(backend).unwrap();

        terminal.draw(|frame| render(frame, &session)).unwrap();
        let rendered = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>();

        assert!(rendered.contains("stream live"));
        assert!(rendered.contains("Ctrl-O input"));
    }

    #[test]
    fn render_fullscreen_logs_hides_fields_panel() {
        let values = parse_values(
            InputKind::Json,
            br#"{"name":"Alice","status":"active"}"#,
            false,
        )
        .unwrap();
        let config = ExploreConfig {
            max_input_bytes: 1024 * 1024,
            max_schema_documents: 10,
            max_preview_results: 10,
            max_schema_depth: 4,
        };
        let mut session = ExploreSession::new(
            values,
            "name".to_owned(),
            String::new(),
            config,
            output::OutputOptions::plain(),
        );
        session.preview_fullscreen = true;
        let backend = TestBackend::new(100, 24);
        let mut terminal = Terminal::new(backend).unwrap();

        terminal.draw(|frame| render(frame, &session)).unwrap();
        let rendered = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>();

        assert!(!rendered.contains("Fields"));
        assert!(rendered.contains("Preview"));
        assert!(rendered.contains("Alice"));
        assert!(rendered.contains("logs:full"));
    }

    #[test]
    fn filter_view_keeps_long_input_cursor_visible() {
        let input = "select(.items[] | .metadata.annotations.owner)";

        let (visible, cursor_column) = filter_view(input, input.len(), 12);

        assert_eq!(visible, "ions.owner)");
        assert_eq!(cursor_column, 11);
    }

    #[test]
    fn unicode_editor_uses_terminal_columns_and_whole_graphemes() {
        let input = "a界👩‍💻e\u{301}z";
        let (visible, cursor) = filter_view(input, input.len(), 6);
        assert!(Line::raw(&visible).width() <= 6);
        assert_eq!(cursor as usize, Line::raw(&visible).width());
        assert_eq!(previous_boundary("e\u{301}", "e\u{301}".len()), 0);
        assert_eq!(next_boundary("👩‍💻x", 0), "👩‍💻".len());
        assert_eq!(pad_preview_line("界", 4), "界  ");
    }

    #[test]
    fn seeded_wrapping_preserves_text_and_never_exceeds_terminal_width() {
        let mut seed = 0x57524150_u64;
        let symbols = ["a", " ", "界", "👩‍💻", "e\u{301}", "🦀", "["];
        for _ in 0..300 {
            seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
            let width = (seed % 50 + 2) as u16;
            let mut text = String::new();
            for _ in 0..seed % 180 + 1 {
                seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
                text.push_str(symbols[(seed >> 32) as usize % symbols.len()]);
            }
            let rows = wrap_text(&text, width);
            assert_eq!(rows.concat(), text);
            assert!(rows
                .iter()
                .all(|row| Line::raw(row).width() <= usize::from(width)));
        }
    }

    #[test]
    fn wrapped_colored_tail_and_resize_use_physical_scroll_rows() {
        let mut session = ExploreSession::new_streaming(
            String::new(),
            "message".to_owned(),
            ExploreConfig {
                max_input_bytes: 100000,
                max_schema_documents: 2,
                max_preview_results: 3,
                max_schema_depth: 4,
            },
            output::OutputOptions {
                color_level: true,
                no_color: false,
                ..output::OutputOptions::plain()
            },
        );
        for i in 0..6 {
            let input = format!(
                "{{\"level\":\"ERROR\",\"message\":\"{} LAST{i}\"}}",
                "界".repeat(99)
            );
            session.append_stream_values(
                parse_values(InputKind::Json, input.as_bytes(), false).unwrap(),
            );
        }
        for (width, height) in [(40, 20), (120, 30), (26, 18), (80, 24)] {
            update_viewport(&mut session, Rect::new(0, 0, width, height));
            assert!(session
                .preview
                .visual_lines
                .iter()
                .all(|row| row.width() <= usize::from(session.preview.viewport_width)));
            assert_eq!(session.preview.preview_scroll, session.max_preview_scroll());
            let shown = rendered_session(&session, width, height);
            assert!(shown.contains("LAST5"), "{width}x{height}: {shown}");
        }
        update_viewport(&mut session, Rect::new(0, 0, 120, 30));
        assert!(rendered_session(&session, 120, 30).contains("2/2 docs"));
        let frozen = preview_text(&session);
        session.toggle_follow_tail();
        session.append_stream_values(
            parse_values(
                InputKind::Json,
                br#"{"level":"ERROR","message":"NEWEST"}"#,
                false,
            )
            .unwrap(),
        );
        assert_eq!(preview_text(&session), frozen);
        session.toggle_follow_tail();
        update_viewport(&mut session, Rect::new(0, 0, 40, 20));
        assert!(rendered_session(&session, 40, 20).contains("NEWEST"));
    }

    #[test]
    fn small_preview_does_not_scroll_into_empty_space() {
        let mut session = help_test_session();
        update_viewport(&mut session, Rect::new(0, 0, 40, 20));
        session.scroll_preview(100);
        assert_eq!(session.preview.preview_scroll, 0);
    }

    #[test]
    fn empty_live_paused_and_disabled_previews_describe_their_actual_state() {
        let mut session = ExploreSession::new_streaming(
            "name".to_owned(),
            String::new(),
            ExploreConfig {
                max_input_bytes: 1000,
                max_schema_documents: 2,
                max_preview_results: 3,
                max_schema_depth: 4,
            },
            output::OutputOptions::plain(),
        );
        let text = |session: &ExploreSession| {
            preview_render_lines(session, 80)
                .iter()
                .flat_map(|line| line.spans.iter().map(|span| span.content.to_string()))
                .collect::<String>()
        };
        assert!(text(&session).contains("waiting for matching logs"));
        session.toggle_follow_tail();
        session.append_stream_values(
            parse_values(InputKind::Json, br#"{"name":"new"}"#, false).unwrap(),
        );
        assert_eq!(session.preview.displayed_results, 0);
        assert!(text(&session).contains("preview paused"));
        session.preview.limit = 0;
        session.refresh();
        assert!(text(&session).contains("preview disabled"));
        session.preview.limit = 3;
        session.data.values.clear();
        session.refresh();
        session.end_stream();
        assert!(text(&session).contains("no matches; adjust"));
    }

    #[test]
    fn tiny_terminal_sizes_render_without_panics_and_can_be_resized_back() {
        let mut session = help_test_session();
        for width in [1, 2, 8, 10, 15, 20, 26, 40] {
            for height in [1, 2, 8, 10, 16, 20] {
                update_viewport(&mut session, Rect::new(0, 0, width, height));
                let _ = rendered_session(&session, width, height);
            }
        }
        update_viewport(&mut session, Rect::new(0, 0, 120, 30));
        assert!(rendered_session(&session, 120, 30).contains("Alice"));
        assert_eq!(
            handle_key(&mut session, KeyCode::Esc, KeyModifiers::NONE),
            Some(ExplorerExit::Quit)
        );
    }

    #[test]
    fn tail_can_reach_more_than_u16_physical_rows_without_cloning_the_whole_window() {
        let mut session = help_test_session();
        session.stream = Some(StreamState {
            ended: true,
            errors: Vec::new(),
        });
        session.preview.lines = (0..70000)
            .map(|index| PreviewLine {
                text: format!("row-{index}"),
                style: Style::default(),
                badge: None,
            })
            .collect();
        session.preview.viewport_width = 0;
        update_viewport(&mut session, Rect::new(0, 0, 40, 20));
        assert!(session.preview.preview_scroll > usize::from(u16::MAX));
        assert!(rendered_session(&session, 40, 20).contains("row-69999"));
        session.scroll_preview(-10);
        assert!(!session.preview.follow_tail);
        assert_eq!(
            session.preview.preview_scroll,
            session.max_preview_scroll() - 10
        );
    }
}
