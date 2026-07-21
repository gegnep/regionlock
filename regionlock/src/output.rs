//! Human-output rendering: aligned tables and ANSI color.
//!
//! Color applies only when stdout is a terminal, NO_COLOR is unset, and
//! TERM is not "dumb" (SPEC: JSON contract). Piped output stays plain.

use std::io::IsTerminal;

/// Terminal styling decision, detected once per invocation.
#[derive(Debug, Clone, Copy, Default)]
pub struct Style {
    color: bool,
}

impl Style {
    pub fn detect() -> Self {
        let color = std::io::stdout().is_terminal()
            && std::env::var_os("NO_COLOR").is_none()
            && std::env::var_os("TERM").is_none_or(|term| term.to_string_lossy() != "dumb");
        Style { color }
    }

    pub fn bold(&self, text: &str) -> String {
        self.wrap("\x1b[1m", text)
    }

    pub fn red(&self, text: &str) -> String {
        self.wrap("\x1b[31m", text)
    }

    pub fn green(&self, text: &str) -> String {
        self.wrap("\x1b[32m", text)
    }

    fn wrap(&self, code: &str, text: &str) -> String {
        if self.color {
            format!("{code}{text}\x1b[0m")
        } else {
            text.to_string()
        }
    }
}

/// Per-cell color hint for table rows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CellStyle {
    Plain,
    Red,
    Green,
}

/// One table cell: plain text plus a color hint. Widths are computed on the
/// plain text; escape codes wrap the padded cell so alignment survives.
#[derive(Debug, Clone)]
pub struct Cell {
    text: String,
    style: CellStyle,
}

impl Cell {
    pub fn plain(text: impl Into<String>) -> Self {
        Cell {
            text: text.into(),
            style: CellStyle::Plain,
        }
    }

    pub fn red(text: impl Into<String>) -> Self {
        Cell {
            text: text.into(),
            style: CellStyle::Red,
        }
    }

    pub fn green(text: impl Into<String>) -> Self {
        Cell {
            text: text.into(),
            style: CellStyle::Green,
        }
    }
}

/// Render an aligned table: columns padded to the widest cell, two-space
/// column gaps, bold header. The last column is not padded (no trailing
/// whitespace).
pub fn render_table(headers: &[&str], rows: &[Vec<Cell>], style: &Style) -> String {
    let columns = headers.len();
    let mut widths: Vec<usize> = headers.iter().map(|header| header.len()).collect();
    for row in rows {
        assert_eq!(row.len(), columns, "row width must match the header");
        for (index, cell) in row.iter().enumerate() {
            widths[index] = widths[index].max(cell.text.len());
        }
    }
    let pad = |text: &str, index: usize| -> String {
        if index + 1 == columns {
            text.to_string()
        } else {
            format!("{text:<width$}", width = widths[index])
        }
    };
    let mut out = headers
        .iter()
        .enumerate()
        .map(|(index, header)| style.bold(&pad(header, index)))
        .collect::<Vec<_>>()
        .join("  ");
    for row in rows {
        out.push('\n');
        let line = row
            .iter()
            .enumerate()
            .map(|(index, cell)| {
                let padded = pad(&cell.text, index);
                match cell.style {
                    CellStyle::Plain => padded,
                    CellStyle::Red => style.red(&padded),
                    CellStyle::Green => style.green(&padded),
                }
            })
            .collect::<Vec<_>>()
            .join("  ");
        out.push_str(&line);
    }
    out
}
