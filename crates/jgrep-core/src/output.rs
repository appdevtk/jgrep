use std::io::{self, Write};

use jaq_json::Val;

use crate::cli::Cli;
use crate::color;

#[derive(Debug, Clone)]
pub struct OutputOptions {
    pub pretty: bool,
    pub json_color: bool,
    pub color_level: bool,
    pub no_color: bool,
    pub color_level_field: Option<String>,
}

impl OutputOptions {
    pub fn from_cli(cli: &Cli) -> Self {
        Self {
            pretty: cli.pretty,
            json_color: cli.use_json_color(),
            color_level: cli.color_level,
            no_color: cli.no_color,
            color_level_field: cli.color_level_field.clone(),
        }
    }

    #[cfg(test)]
    pub fn plain() -> Self {
        Self {
            pretty: false,
            json_color: false,
            color_level: false,
            no_color: true,
            color_level_field: None,
        }
    }
}

pub fn write_result(
    out: &mut dyn Write,
    result: &Val,
    filename: Option<&str>,
    show_filename: bool,
    source: &Val,
    cli: &Cli,
) -> io::Result<()> {
    let options = OutputOptions::from_cli(cli);
    let output = format_display_value(result, &options)?;

    if show_filename {
        if let Some(filename) = filename {
            let line = format!("{filename}:{output}");
            if let Some(colored) = color::colorize_by_level(
                &line,
                source,
                options.color_level,
                options.no_color,
                options.color_level_field.as_deref(),
            ) {
                writeln!(out, "{colored}")?;
            } else {
                writeln!(out, "{line}")?;
            }
        } else {
            writeln!(out, "{output}")?;
        }
    } else {
        writeln!(
            out,
            "{}",
            colorize_level_if_needed(&output, source, &options)
        )?;
    }
    Ok(())
}

pub fn format_result(result: &Val, source: &Val, options: &OutputOptions) -> io::Result<String> {
    let output = format_display_value(result, options)?;
    Ok(colorize_level_if_needed(&output, source, options))
}

fn format_display_value(result: &Val, options: &OutputOptions) -> io::Result<String> {
    let mut output = format_value(result, options.pretty)?;
    if options.json_color && !matches!(result, Val::TStr(_)) {
        output = color::colorize_json(&output);
    }
    Ok(output)
}

fn colorize_level_if_needed(output: &str, source: &Val, options: &OutputOptions) -> String {
    if let Some(colored) = color::colorize_by_level(
        output,
        source,
        options.color_level,
        options.no_color,
        options.color_level_field.as_deref(),
    ) {
        colored
    } else {
        output.to_owned()
    }
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
