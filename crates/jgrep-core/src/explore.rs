use std::io::{self, BufRead, IsTerminal, Read, Write};
use std::path::Path;
use std::sync::mpsc::{self, Receiver};
use std::thread;
use std::time::Duration;

use crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use crossterm::execute;
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use jaq_json::Val;
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Constraint, Direction, Layout, Position};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, List, ListItem, Paragraph, Wrap};
use ratatui::{Frame, Terminal};

use crate::cli::ExploreArgs;
use crate::input::{self, InputKind};
use crate::matcher::{self, Matcher};
use crate::schema::{self, FieldSummary};
use crate::{color, output, shortcuts, NAME};

pub fn run(args: ExploreArgs) -> i32 {
    let mut err = io::BufWriter::new(io::stderr());
    let config = ExploreConfig::from_args(&args);
    let filter = initial_filter_input(&args);
    let output_input = args.path_filter.clone().unwrap_or_default();
    let output_options = output_options_from_args(&args);

    if args.input.is_none()
        && !args.print
        && io::stdout().is_terminal()
        && !io::stdin().is_terminal()
    {
        match prepare_stdin_tui(config.max_input_bytes) {
            Ok(StdinTuiInput::Streaming(first_line)) => {
                let stream = spawn_json_line_stream(first_line, config.max_input_bytes);
                let mut session =
                    ExploreSession::new_streaming(filter, output_input, config, output_options);
                return finish_tui(
                    run_tui_streaming(&mut session, &stream),
                    &mut session,
                    &mut err,
                );
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
                return finish_tui(run_tui(&mut session), &mut session, &mut err);
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
        let has_error = session.filter.error.is_some();
        if print_snapshot(&session, &mut out).is_err() {
            return 2;
        }
        return if has_error { 2 } else { 0 };
    }

    finish_tui(run_tui(&mut session), &mut session, &mut err)
}

fn initial_filter_input(args: &ExploreArgs) -> String {
    if let Some(filter) = &args.filter {
        return filter.clone();
    }
    if args.where_filters.is_empty() {
        String::new()
    } else {
        shortcuts::where_filters_to_select(&args.where_filters).unwrap_or_default()
    }
}

fn output_options_from_args(args: &ExploreArgs) -> output::OutputOptions {
    output::OutputOptions {
        pretty: args.pretty,
        json_color: false,
        color_level: args.color_level,
        no_color: args.no_color,
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
    let mut reader = io::BufReader::new(stdin.lock());
    let mut first_line = Vec::new();
    reader
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
        .take(max_input_bytes.saturating_sub(bytes.len()) as u64 + 1)
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
            match reader.read_until(b'\n', &mut line) {
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
            let bytes = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
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
                .take(max_input_bytes as u64 + 1)
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

    if filter_input.starts_with("select(") || filter_input.starts_with('.') {
        Ok(format!("{filter_input} | {output}"))
    } else {
        shortcuts::apply_where_filters(output, &[filter_input.to_owned()])
    }
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
    completions: Vec<String>,
    completion_index: usize,
    completion_prefix: String,
}

struct PreviewState {
    lines: Vec<PreviewLine>,
    match_count: usize,
    limit: usize,
    field_scroll: usize,
    preview_scroll: u16,
}

#[derive(Clone)]
struct PreviewLine {
    text: String,
    style: Style,
}

struct ExplorerPersistence {
    last_filter_file: Option<std::ffi::OsString>,
}

struct ExploreSession {
    data: ExplorerData,
    filter: FilterState,
    output: TextState,
    active_input: ActiveInput,
    preview_fullscreen: bool,
    output_options: output::OutputOptions,
    preview: PreviewState,
    persistence: ExplorerPersistence,
    config: ExploreConfig,
    stream: Option<StreamState>,
    message: Option<String>,
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
        let fields = schema::infer_schema(
            &values,
            config.max_schema_documents,
            config.max_schema_depth,
        );
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
            output_options,
            preview: PreviewState {
                lines: Vec::new(),
                match_count: 0,
                limit: config.max_preview_results,
                field_scroll: 0,
                preview_scroll: 0,
            },
            persistence: ExplorerPersistence {
                last_filter_file: std::env::var_os("JGREP_LAST_FILTER_FILE"),
            },
            config,
            stream: None,
            message: None,
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
        session.message = Some("streaming stdin: waiting for JSON log lines".to_owned());
        session
    }

    fn append_stream_value(&mut self, value: Val) {
        self.data.values.push(value);
        self.data.fields = schema::infer_schema(
            &self.data.values,
            self.config.max_schema_documents,
            self.config.max_schema_depth,
        );
        self.refresh();
    }

    fn record_stream_error(&mut self, error: String) {
        if let Some(stream) = &mut self.stream {
            stream.errors.push(error);
        }
    }

    fn end_stream(&mut self) {
        if let Some(stream) = &mut self.stream {
            stream.ended = true;
        }
    }

    fn refresh(&mut self) {
        match compose_explore_jq(&self.filter.text.input, &self.output.input) {
            Ok(generated) => {
                self.filter.generated = generated;
                self.persist_last_filter();
                match preview_tui_values(
                    &self.data.values,
                    &self.filter.generated,
                    self.preview.limit,
                    &self.output_options,
                ) {
                    Ok(preview) => {
                        self.preview.lines = preview.lines;
                        self.preview.match_count = preview.match_count;
                        self.clamp_preview_scroll();
                        self.filter.error = None;
                    }
                    Err(e) => {
                        self.preview.lines.clear();
                        self.preview.match_count = 0;
                        self.filter.error = Some(e);
                    }
                }
            }
            Err(e) => {
                self.filter.generated.clear();
                self.preview.lines.clear();
                self.preview.match_count = 0;
                self.filter.error = Some(e);
            }
        }
    }

    fn autocomplete_filter(&mut self) {
        let active_input = self.active_text().input.clone();
        if !self.filter.completions.is_empty()
            && self
                .filter
                .completions
                .get(self.filter.completion_index)
                .is_some_and(|candidate| candidate == &active_input)
        {
            self.filter.completion_index =
                (self.filter.completion_index + 1) % self.filter.completions.len();
            let replacement = self.filter.completions[self.filter.completion_index].clone();
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
            .map(|field| field.path.clone())
            .collect();
        self.filter.completion_index = 0;
        self.filter.completion_prefix = prefix;

        if let Some(field) = self.filter.completions.first() {
            self.active_text_mut().input = field.clone();
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
            .map(String::as_str)
            .unwrap_or("");
        let candidates = self
            .filter
            .completions
            .iter()
            .take(5)
            .map(|path| {
                let types = self
                    .data
                    .fields
                    .iter()
                    .find(|field| field.path == *path)
                    .map(|field| field.types.join("|"))
                    .unwrap_or_default();
                if types.is_empty() {
                    path.clone()
                } else {
                    format!("{path} [{types}]")
                }
            })
            .collect::<Vec<_>>()
            .join(", ");
        format!("completion {index}/{total}: {current} | {candidates}")
    }

    fn save_filter(&mut self) {
        if self.filter.generated.is_empty() {
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
        let max = self.preview.lines.len().saturating_sub(1) as u16;
        let next = if delta < 0 {
            self.preview
                .preview_scroll
                .saturating_sub(delta.unsigned_abs() as u16)
        } else {
            self.preview.preview_scroll.saturating_add(delta as u16)
        };
        self.preview.preview_scroll = next.min(max);
    }

    fn clamp_preview_scroll(&mut self) {
        let max = self.preview.lines.len().saturating_sub(1) as u16;
        self.preview.preview_scroll = self.preview.preview_scroll.min(max);
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
            "level-color:on"
        } else {
            "level-color:off"
        };
        let scroll = if self.preview.lines.is_empty() {
            "scroll:0/0".to_owned()
        } else {
            format!(
                "scroll:{}/{}",
                self.preview.preview_scroll.saturating_add(1),
                self.preview.lines.len()
            )
        };
        let output = if self.output.input.trim().is_empty() {
            "."
        } else {
            self.output.input.trim()
        };
        let layout = if self.preview_fullscreen {
            "logs:full"
        } else {
            "logs:split"
        };
        let mut status = format!(
            "{} matches | editing:{active} | {pretty} | {level} | {layout} | {scroll} | output:{output}",
            self.preview.match_count
        );
        if let Some(message) = &self.message {
            status.push_str(" | ");
            status.push_str(message);
        }
        status
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
    input[..cursor]
        .char_indices()
        .last()
        .map_or(0, |(idx, _)| idx)
}

fn next_boundary(input: &str, cursor: usize) -> usize {
    input[cursor..]
        .char_indices()
        .nth(1)
        .map_or(input.len(), |(idx, _)| cursor + idx)
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

fn preview_values(
    values: &[Val],
    filter: &str,
    limit: usize,
    options: &output::OutputOptions,
) -> Result<Vec<String>, String> {
    let matcher = Matcher::compile(filter)?;
    let mut lines = Vec::new();

    for value in values {
        for result in matcher.apply(value.clone())? {
            if !matcher::is_match(&result) {
                continue;
            }
            lines.push(output::format_result(&result, value, options).map_err(|e| e.to_string())?);
            if lines.len() >= limit {
                return Ok(lines);
            }
        }
    }

    Ok(lines)
}

struct TuiPreview {
    lines: Vec<PreviewLine>,
    match_count: usize,
}

fn preview_tui_values(
    values: &[Val],
    filter: &str,
    limit: usize,
    options: &output::OutputOptions,
) -> Result<TuiPreview, String> {
    let matcher = Matcher::compile(filter)?;
    let mut lines = Vec::new();
    let mut match_count = 0;
    let display_options = output::OutputOptions {
        pretty: options.pretty,
        json_color: false,
        color_level: false,
        no_color: true,
        color_level_field: options.color_level_field.clone(),
    };

    for value in values {
        for result in matcher.apply(value.clone())? {
            if !matcher::is_match(&result) {
                continue;
            }
            match_count += 1;
            let text = output::format_result(&result, value, &display_options)
                .map_err(|e| e.to_string())?;
            let style = preview_line_style(value, options);
            for line in text.lines() {
                lines.push(PreviewLine {
                    text: line.to_owned(),
                    style,
                });
            }
            if text.is_empty() {
                lines.push(PreviewLine {
                    text: String::new(),
                    style,
                });
            }
            if match_count >= limit {
                return Ok(TuiPreview { lines, match_count });
            }
        }
    }
    Ok(TuiPreview { lines, match_count })
}

fn preview_line_style(source: &Val, options: &output::OutputOptions) -> Style {
    match color::color_code_by_level_without_env(
        source,
        options.color_level,
        options.no_color,
        options.color_level_field.as_deref(),
    ) {
        Some("\u{1b}[36m") => Style::default().fg(Color::Black).bg(Color::Cyan),
        Some("\u{1b}[34m") => Style::default().fg(Color::White).bg(Color::Blue),
        Some("\u{1b}[33m") => Style::default().fg(Color::Black).bg(Color::Yellow),
        Some("\u{1b}[35m") => Style::default().fg(Color::White).bg(Color::Magenta),
        Some("\u{1b}[31m") => Style::default().fg(Color::White).bg(Color::Red),
        Some("\u{1b}[1;31m") => Style::default()
            .fg(Color::White)
            .bg(Color::Red)
            .add_modifier(Modifier::BOLD),
        _ => Style::default(),
    }
}

fn preview_render_lines(session: &ExploreSession, width: u16) -> Vec<Line<'static>> {
    if let Some(error) = &session.filter.error {
        return vec![Line::styled(
            pad_preview_line(&format!("error: {error}"), width),
            Style::default().fg(Color::Red),
        )];
    }
    if session.preview.lines.is_empty() {
        return vec![Line::from("no matches")];
    }

    session
        .preview
        .lines
        .iter()
        .map(|line| {
            let text = if line.style.bg.is_some() {
                pad_preview_line(&line.text, width)
            } else {
                line.text.clone()
            };
            Line::styled(text, line.style)
        })
        .collect()
}

fn pad_preview_line(text: &str, width: u16) -> String {
    let width = width as usize;
    let len = text.chars().count();
    if len >= width {
        text.to_owned()
    } else {
        format!("{text}{}", " ".repeat(width - len))
    }
}

fn print_snapshot(session: &ExploreSession, out: &mut dyn Write) -> io::Result<()> {
    writeln!(out, "jgrep explore")?;
    writeln!(
        out,
        "filter: {}",
        if session.filter.text.input.trim().is_empty() {
            "."
        } else {
            session.filter.text.input.trim()
        }
    )?;
    writeln!(
        out,
        "output: {}",
        if session.output.input.trim().is_empty() {
            "."
        } else {
            session.output.input.trim()
        }
    )?;
    writeln!(out, "generated jq: {}", session.filter.generated)?;
    writeln!(out)?;
    writeln!(out, "fields:")?;
    for field in session.data.fields.iter().take(25) {
        let optional = if field.optional { " optional" } else { "" };
        let examples = if field.examples.is_empty() {
            String::new()
        } else {
            format!(" examples={}", field.examples.join(","))
        };
        writeln!(
            out,
            "  {}  {}  ({}, docs={}{}{examples})",
            field.path,
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
        writeln!(out, "  error: {error}")?;
    } else {
        let preview = preview_values(
            &session.data.values,
            &session.filter.generated,
            session.preview.limit,
            &session.output_options,
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
    if let Some(error) = &session.filter.error {
        eprintln!("{NAME}: explore: {error}");
        return 2;
    }
    let results = match preview_values(
        &session.data.values,
        &session.filter.generated,
        usize::MAX,
        &session.output_options,
    ) {
        Ok(results) => results,
        Err(e) => {
            eprintln!("{NAME}: explore: {e}");
            return 2;
        }
    };
    for line in &results {
        println!("{line}");
    }
    if results.is_empty() {
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

fn run_tui(session: &mut ExploreSession) -> io::Result<ExplorerExit> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let result = run_tui_loop(&mut terminal, session, None);
    let raw_mode_result = disable_raw_mode();
    let screen_result = execute!(terminal.backend_mut(), LeaveAlternateScreen);
    let cursor_result = terminal.show_cursor();

    let exit = result?;
    raw_mode_result?;
    screen_result?;
    cursor_result?;
    Ok(exit)
}

fn run_tui_streaming(
    session: &mut ExploreSession,
    stream: &Receiver<StreamEvent>,
) -> io::Result<ExplorerExit> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let result = run_tui_loop(&mut terminal, session, Some(stream));
    let raw_mode_result = disable_raw_mode();
    let screen_result = execute!(terminal.backend_mut(), LeaveAlternateScreen);
    let cursor_result = terminal.show_cursor();

    let exit = result?;
    raw_mode_result?;
    screen_result?;
    cursor_result?;
    Ok(exit)
}

fn run_tui_loop(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    session: &mut ExploreSession,
    stream: Option<&Receiver<StreamEvent>>,
) -> io::Result<ExplorerExit> {
    let result = loop {
        if let Some(stream) = stream {
            drain_stream(session, stream);
        }

        terminal.draw(|frame| render(frame, session))?;

        if !event::poll(Duration::from_millis(150))? {
            continue;
        }

        let Event::Key(key) = event::read()? else {
            continue;
        };
        if key.kind != KeyEventKind::Press {
            continue;
        }

        if let Some(exit) = handle_key(session, key.code, key.modifiers) {
            break exit;
        }
    };
    Ok(result)
}

fn drain_stream(session: &mut ExploreSession, stream: &Receiver<StreamEvent>) {
    let mut added = 0usize;
    let mut ended = false;
    for event in stream.try_iter() {
        match event {
            StreamEvent::Line(line) => match parse_stream_line(&line) {
                Ok(values) => {
                    for value in values {
                        session.append_stream_value(value);
                        added += 1;
                    }
                }
                Err(error) => session.record_stream_error(error),
            },
            StreamEvent::Error(error) => session.record_stream_error(error),
            StreamEvent::End => {
                session.end_stream();
                ended = true;
            }
        }
    }
    if added > 0 || ended {
        let status = if session.stream.as_ref().is_some_and(|stream| stream.ended) {
            "complete"
        } else {
            "live"
        };
        session.message = Some(format!(
            "streaming stdin: {} documents ({status})",
            session.data.values.len()
        ));
    }
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
    match code {
        KeyCode::Enter => {
            session.record_history();
            Some(ExplorerExit::PrintResults)
        }
        KeyCode::Esc => Some(ExplorerExit::Quit),
        KeyCode::Char('c') if modifiers.contains(KeyModifiers::CONTROL) => Some(ExplorerExit::Quit),
        KeyCode::Char('y') if modifiers.contains(KeyModifiers::CONTROL) => {
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
            if session.active_input == ActiveInput::Output {
                session.scroll_preview(-1);
            } else {
                session.scroll_fields(-1);
            }
            None
        }
        KeyCode::Down => {
            if session.active_input == ActiveInput::Output {
                session.scroll_preview(1);
            } else {
                session.scroll_fields(1);
            }
            None
        }
        KeyCode::PageUp => {
            session.scroll_preview(-10);
            None
        }
        KeyCode::PageDown => {
            session.scroll_preview(10);
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

fn filter_view(input: &str, cursor: usize, width: u16) -> (String, u16) {
    let width = width as usize;
    if width == 0 {
        return (String::new(), 0);
    }

    let cursor_char = input[..cursor].chars().count();
    let start_char = cursor_char.saturating_sub(width.saturating_sub(1));
    let visible = input
        .chars()
        .skip(start_char)
        .take(width)
        .collect::<String>();
    let cursor_column = cursor_char
        .saturating_sub(start_char)
        .min(width.saturating_sub(1)) as u16;
    (visible, cursor_column)
}

fn render(frame: &mut Frame, session: &ExploreSession) {
    let outer = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(6),
            Constraint::Min(5),
            Constraint::Length(5),
        ])
        .split(frame.area());

    let inputs = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(3), Constraint::Length(3)])
        .split(outer[0]);

    let body = if session.preview_fullscreen {
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
    frame.set_cursor_position(Position::new(cursor_x, cursor_area.y + 1));

    let preview_area = if session.preview_fullscreen {
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
                        session.data.values.len(),
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

    let preview_lines = preview_render_lines(session, preview_area.width.saturating_sub(2));
    frame.render_widget(
        Paragraph::new(preview_lines)
            .wrap(Wrap { trim: false })
            .scroll((session.preview.preview_scroll, 0))
            .block(Block::default().title("Preview").borders(Borders::ALL)),
        preview_area,
    );

    let status = session.status_line();
    let stream_status = session.stream.as_ref().map(|stream| {
        let status = if stream.ended { "complete" } else { "live" };
        format!(
            "stream {status}: {} docs, {} errors",
            session.data.values.len(),
            stream.errors.len()
        )
    });
    let hint = if let Some(stream_status) = stream_status {
        format!(
            "{status}\n{stream_status}\nCtrl-O input | Ctrl-G logs | Tab complete | Ctrl-F pretty | Ctrl-L level color | Ctrl-K color off/on | Ctrl-P/N history | Enter print | Ctrl-Y jq | Ctrl-S save | Esc quit"
        )
    } else {
        format!(
            "{status}\nCtrl-O input | Ctrl-G logs | Tab complete | Ctrl-F pretty | Ctrl-L level color | Ctrl-K color off/on | Ctrl-P/N history | Enter print | Ctrl-Y jq | Ctrl-S save | Esc quit"
        )
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::backend::TestBackend;

    fn preview_text(session: &ExploreSession) -> Vec<String> {
        session
            .preview
            .lines
            .iter()
            .map(|line| line.text.clone())
            .collect()
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
            .any(|line| line.style.bg == Some(Color::Red)));
    }

    #[test]
    fn render_level_color_uses_visible_line_background() {
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
            .any(|cell| cell.symbol() == "b" && cell.bg == Color::Red));
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

        assert_eq!(session.filter.text.input, "name");
        assert!(session
            .message
            .as_deref()
            .unwrap_or("")
            .contains("[string]"));

        handle_key(&mut session, KeyCode::Tab, KeyModifiers::NONE);

        assert_eq!(session.filter.text.input, "namespace");
        assert!(session
            .message
            .as_deref()
            .unwrap_or("")
            .contains("completion 2/3"));
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
        assert!(rendered.contains("Tab complete"));
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
}
