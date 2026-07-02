use std::io::{self, Write};

use jaq_json::Val;

use crate::cli::Cli;
use crate::color;

pub fn write_result(
    out: &mut dyn Write,
    result: &Val,
    filename: Option<&str>,
    show_filename: bool,
    source: &Val,
    cli: &Cli,
) -> io::Result<()> {
    let mut output = format_value(result, cli.pretty)?;
    if cli.use_json_color() && !matches!(result, Val::TStr(_)) {
        output = color::colorize_json(&output);
    }

    if show_filename {
        if let Some(filename) = filename {
            let line = format!("{filename}:{output}");
            if let Some(colored) = color::colorize_by_level(&line, source, cli) {
                writeln!(out, "{colored}")?;
            } else {
                writeln!(out, "{line}")?;
            }
        } else {
            writeln!(out, "{output}")?;
        }
    } else if let Some(colored) = color::colorize_by_level(&output, source, cli) {
        writeln!(out, "{colored}")?;
    } else {
        writeln!(out, "{output}")?;
    }
    Ok(())
}

pub fn write_slurp(
    out: &mut dyn Write,
    values: &[Val],
    pretty: bool,
    color: bool,
) -> io::Result<()> {
    let array = Val::Arr(values.to_vec().into());
    let mut output = format_value(&array, pretty)?;
    if color {
        output = crate::color::colorize_json(&output);
    }
    writeln!(out, "{output}")
}

pub fn format_value(value: &Val, pretty: bool) -> io::Result<String> {
    if let Val::TStr(bytes) = value {
        return Ok(String::from_utf8_lossy(bytes).into_owned());
    }

    let mut buf = Vec::new();
    let pp = jaq_json::write::Pp {
        indent: pretty.then(|| "  ".to_owned()),
        sep_space: pretty,
        ..Default::default()
    };
    jaq_json::write::write(&mut buf, &pp, 0, value)?;
    String::from_utf8(buf).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
}
