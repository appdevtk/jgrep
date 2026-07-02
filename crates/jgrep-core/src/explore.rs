use std::io::{self, IsTerminal, Read, Write};
use std::path::Path;
use std::time::Duration;

use crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use crossterm::execute;
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use jaq_json::Val;
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Constraint, Direction, Layout};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, List, ListItem, Paragraph, Wrap};
use ratatui::{Frame, Terminal};

use crate::cli::ExploreArgs;
use crate::input::{self, InputKind};
use crate::matcher::{self, Matcher};
use crate::schema::{self, FieldSummary};
use crate::{output, shortcuts, NAME};

const MAX_SCHEMA_DOCUMENTS: usize = 500;
const MAX_PREVIEW_RESULTS: usize = 30;

pub fn run(args: ExploreArgs) -> i32 {
    let mut err = io::BufWriter::new(io::stderr());
    let values = match read_values(args.input.as_deref()) {
        Ok(values) => values,
        Err(e) => {
            let _ = writeln!(err, "{NAME}: explore: {e}");
            let _ = err.flush();
            return 2;
        }
    };

    let mut session = ExploreSession::new(values, args.filter.unwrap_or_default());
    if args.print || !io::stdout().is_terminal() {
        let mut out = io::BufWriter::new(io::stdout());
        let has_error = session.error.is_some();
        if print_snapshot(&session, &mut out).is_err() {
            return 2;
        }
        return if has_error { 2 } else { 0 };
    }

    match run_tui(&mut session) {
        Ok(ExplorerExit::PrintResults) => print_results(&session),
        Ok(ExplorerExit::PrintFilter) => {
            println!("{}", session.generated);
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

pub fn read_values(path: Option<&Path>) -> Result<Vec<Val>, String> {
    match path {
        Some(path) => {
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
            let mut bytes = Vec::new();
            io::stdin()
                .read_to_end(&mut bytes)
                .map_err(|e| format!("stdin: {e}"))?;
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

struct ExploreSession {
    values: Vec<Val>,
    fields: Vec<FieldSummary>,
    filter: String,
    generated: String,
    preview: Vec<String>,
    error: Option<String>,
    message: Option<String>,
}

impl ExploreSession {
    fn new(values: Vec<Val>, filter: String) -> Self {
        let fields = schema::infer_schema(&values, MAX_SCHEMA_DOCUMENTS);
        let mut session = Self {
            values,
            fields,
            filter,
            generated: String::new(),
            preview: Vec::new(),
            error: None,
            message: None,
        };
        session.refresh();
        session
    }

    fn refresh(&mut self) {
        match shortcuts::expression_to_jq(&self.filter) {
            Ok(generated) => {
                self.generated = generated;
                self.persist_last_filter();
                match preview_values(&self.values, &self.generated, MAX_PREVIEW_RESULTS) {
                    Ok(preview) => {
                        self.preview = preview;
                        self.error = None;
                    }
                    Err(e) => {
                        self.preview.clear();
                        self.error = Some(e);
                    }
                }
            }
            Err(e) => {
                self.generated.clear();
                self.preview.clear();
                self.error = Some(e);
            }
        }
    }

    fn autocomplete_filter(&mut self) {
        let prefix = self.filter.trim();
        if prefix.is_empty() || prefix.starts_with('.') || prefix.starts_with("select(") {
            return;
        }

        if let Some(field) = self
            .fields
            .iter()
            .find(|field| field.path.starts_with(prefix))
        {
            self.filter = field.path.clone();
            self.message = Some(format!("completed {}", field.path));
            self.refresh();
        }
    }

    fn save_filter(&mut self) {
        if self.generated.is_empty() {
            self.message = Some("no valid filter to save".to_owned());
            return;
        }

        let path = std::env::var("JGREP_FILTER_SAVE_PATH")
            .unwrap_or_else(|_| "jgrep-filter.jq".to_owned());
        match std::fs::write(&path, format!("{}\n", self.generated)) {
            Ok(()) => self.message = Some(format!("saved {path}")),
            Err(e) => self.message = Some(format!("save failed: {e}")),
        }
    }

    fn persist_last_filter(&self) {
        let Some(path) = std::env::var_os("JGREP_LAST_FILTER_FILE") else {
            return;
        };
        let _ = std::fs::write(path, format!("{}\n", self.generated));
    }
}

fn preview_values(values: &[Val], filter: &str, limit: usize) -> Result<Vec<String>, String> {
    let matcher = Matcher::compile(filter)?;
    let mut lines = Vec::new();

    for value in values {
        for result in matcher.apply(value.clone())? {
            if !matcher::is_match(&result) {
                continue;
            }
            lines.push(output::format_value(&result, false).map_err(|e| e.to_string())?);
            if lines.len() >= limit {
                return Ok(lines);
            }
        }
    }

    Ok(lines)
}

fn print_snapshot(session: &ExploreSession, out: &mut dyn Write) -> io::Result<()> {
    writeln!(out, "jgrep explore")?;
    writeln!(
        out,
        "filter: {}",
        if session.filter.trim().is_empty() {
            "."
        } else {
            session.filter.trim()
        }
    )?;
    writeln!(out, "generated jq: {}", session.generated)?;
    writeln!(out)?;
    writeln!(out, "fields:")?;
    for field in session.fields.iter().take(25) {
        writeln!(
            out,
            "  {}  {}  ({})",
            field.path,
            field.types.join("|"),
            field.count
        )?;
    }
    if session.fields.len() > 25 {
        writeln!(out, "  ... {} more", session.fields.len() - 25)?;
    }
    writeln!(out)?;
    writeln!(out, "preview:")?;
    if let Some(error) = &session.error {
        writeln!(out, "  error: {error}")?;
    } else if session.preview.is_empty() {
        writeln!(out, "  no matches")?;
    } else {
        for line in &session.preview {
            writeln!(out, "  {line}")?;
        }
    }
    Ok(())
}

fn print_results(session: &ExploreSession) -> i32 {
    if let Some(error) = &session.error {
        eprintln!("{NAME}: explore: {error}");
        return 2;
    }
    let results = match preview_values(&session.values, &session.generated, usize::MAX) {
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

    let result = run_tui_loop(&mut terminal, session);
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
) -> io::Result<ExplorerExit> {
    let result = loop {
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

        match key.code {
            KeyCode::Enter => break ExplorerExit::PrintResults,
            KeyCode::Esc => break ExplorerExit::Quit,
            KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                break ExplorerExit::Quit
            }
            KeyCode::Char('y') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                break ExplorerExit::PrintFilter
            }
            KeyCode::Char('s') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                session.save_filter();
            }
            KeyCode::Tab => {
                session.autocomplete_filter();
            }
            KeyCode::Backspace => {
                session.filter.pop();
                session.message = None;
                session.refresh();
            }
            KeyCode::Char(ch) => {
                session.filter.push(ch);
                session.message = None;
                session.refresh();
            }
            _ => {}
        }
    };
    Ok(result)
}

fn render(frame: &mut Frame, session: &ExploreSession) {
    let outer = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Min(5),
            Constraint::Length(3),
        ])
        .split(frame.area());

    let body = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(36), Constraint::Percentage(64)])
        .split(outer[1]);

    let filter = Paragraph::new(session.filter.as_str())
        .block(Block::default().title("Filter").borders(Borders::ALL));
    frame.render_widget(filter, outer[0]);

    let fields = session
        .fields
        .iter()
        .take(body[0].height.saturating_sub(2) as usize)
        .map(|field| {
            ListItem::new(Line::from(vec![
                Span::styled(
                    field.path.clone(),
                    Style::default().add_modifier(Modifier::BOLD),
                ),
                Span::raw(format!("  {}", field.types.join("|"))),
            ]))
        })
        .collect::<Vec<_>>();
    frame.render_widget(
        List::new(fields).block(Block::default().title("Fields").borders(Borders::ALL)),
        body[0],
    );

    let preview_text = if let Some(error) = &session.error {
        format!("error: {error}")
    } else if session.preview.is_empty() {
        "no matches".to_owned()
    } else {
        session.preview.join("\n")
    };
    frame.render_widget(
        Paragraph::new(preview_text)
            .wrap(Wrap { trim: false })
            .block(Block::default().title("Preview").borders(Borders::ALL)),
        body[1],
    );

    let hint = session.message.clone().unwrap_or_else(|| {
        format!(
            "{} preview matches | generated: {} | Tab complete | Enter print result | Ctrl-Y print jq | Ctrl-S save | Esc quit",
            session.preview.len(),
            session.generated
        )
    });
    frame.render_widget(
        Paragraph::new(hint)
            .wrap(Wrap { trim: true })
            .block(Block::default().borders(Borders::ALL)),
        outer[2],
    );
}

#[cfg(test)]
mod tests {
    use super::*;

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

        let preview = preview_values(&values, &jq, 10).unwrap();

        assert_eq!(preview, [r#"{"name":"Alice","status":"active"}"#]);
    }

    #[test]
    fn unknown_extension_can_fall_back_to_yaml() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("selection");
        std::fs::write(&file, "name: Alice\n").unwrap();

        let values = read_values(Some(&file)).unwrap();
        let jq = shortcuts::expression_to_jq("name").unwrap();
        let preview = preview_values(&values, &jq, 10).unwrap();

        assert_eq!(preview, ["Alice"]);
    }
}
