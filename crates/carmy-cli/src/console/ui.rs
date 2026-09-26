//! Draws the console. Pure function of `App`; tested with ratatui's `TestBackend`.
use super::app::{App, Focus, Input, Modal, Phase};
use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    style::{Color, Modifier, Style, Stylize},
    text::{Line, Span, Text},
    widgets::{Block, Clear, List, ListItem, ListState, Paragraph, Wrap},
};

fn effect_badge(effect: &str) -> Span<'static> {
    let (label, color) = match effect {
        "none" => (" N ", Color::Gray),
        "read" => (" R ", Color::Green),
        "write" => (" W ", Color::Yellow),
        "external_write" => (" X ", Color::LightRed),
        "destructive" => (" D ", Color::Red),
        _ => (" ? ", Color::Gray),
    };
    Span::styled(
        label,
        Style::new()
            .fg(Color::Black)
            .bg(color)
            .add_modifier(Modifier::BOLD),
    )
}

fn border(title: &str, focused: bool) -> Block<'_> {
    let style = if focused {
        Style::new().fg(Color::Cyan)
    } else {
        Style::new().fg(Color::DarkGray)
    };
    Block::bordered()
        .title(format!(" {title} "))
        .border_style(style)
}

fn status_style(status: &str) -> Style {
    match status {
        "completed" => Style::new().fg(Color::Green),
        "cancelled" => Style::new().fg(Color::Yellow),
        _ => Style::new().fg(Color::Red),
    }
    .add_modifier(Modifier::BOLD)
}

pub fn draw(frame: &mut Frame, app: &App) {
    let [body, status, footer] = Layout::vertical([
        Constraint::Min(0),
        Constraint::Length(1),
        Constraint::Length(1),
    ])
    .areas(frame.area());
    match &app.phase {
        Phase::Ready => draw_console(frame, app, body),
        Phase::Starting => draw_logs(frame, app, body, "Starting", Color::Cyan),
        Phase::Failed(reason) => draw_logs(frame, app, body, reason, Color::Red),
    }
    let hints = match app.phase {
        Phase::Ready => {
            " ↑↓ tool · Tab focus · Enter run · c confirm · r rerun · h history · ? help · q quit "
        }
        _ => " q quit ",
    };
    frame.render_widget(
        Line::from(Span::styled(
            format!(" {}", app.status),
            Style::new().fg(Color::Cyan),
        )),
        status,
    );
    frame.render_widget(
        Line::from(Span::styled(hints, Style::new().fg(Color::DarkGray))),
        footer,
    );
    match &app.modal {
        Some(Modal::Help) => draw_help(frame),
        Some(Modal::Confirm { tool, grant }) => draw_confirm(frame, tool, *grant),
        Some(Modal::History { selected }) => draw_history(frame, app, *selected),
        None => {}
    }
}

fn draw_logs(frame: &mut Frame, app: &App, area: Rect, title: &str, color: Color) {
    let height = area.height.saturating_sub(2) as usize;
    let lines: Vec<Line> = app
        .logs
        .iter()
        .skip(app.logs.len().saturating_sub(height))
        .map(|l| Line::raw(l.clone()))
        .collect();
    let block = Block::bordered()
        .title(format!(" {title} "))
        .border_style(Style::new().fg(color));
    frame.render_widget(
        Paragraph::new(lines)
            .block(block)
            .wrap(Wrap { trim: false }),
        area,
    );
}

fn draw_console(frame: &mut Frame, app: &App, area: Rect) {
    let [top, inputs, result] = Layout::vertical([
        Constraint::Length(8),
        Constraint::Length(3),
        Constraint::Min(5),
    ])
    .areas(area);
    let [tools_area, details_area] =
        Layout::horizontal([Constraint::Length(34), Constraint::Min(20)]).areas(top);
    let [args_area, id_area] =
        Layout::horizontal([Constraint::Min(20), Constraint::Length(28)]).areas(inputs);

    let items: Vec<ListItem> = app
        .tools
        .iter()
        .map(|tool| {
            let mut spans = vec![
                effect_badge(&tool.effect),
                Span::raw(" "),
                Span::raw(tool.name.clone()),
            ];
            if app.locked(tool) {
                spans.push(Span::styled(" 🔒", Style::new().fg(Color::Red)));
            } else if tool.confirmation {
                spans.push(Span::styled(" ✓", Style::new().fg(Color::Green)));
            }
            ListItem::new(Line::from(spans))
        })
        .collect();
    let mut state = ListState::default().with_selected(Some(app.selected));
    frame.render_stateful_widget(
        List::new(items)
            .block(border(
                &format!("{} tools", app.server),
                app.focus == Focus::Tools,
            ))
            .highlight_style(Style::new().add_modifier(Modifier::REVERSED))
            .highlight_symbol("▶ "),
        tools_area,
        &mut state,
    );

    if let Some(tool) = app.tool() {
        let mut flags = vec![format!("effect: {}", tool.effect)];
        if tool.idempotent {
            flags.push("idempotent".into());
        }
        if tool.parallel_safe {
            flags.push("parallel_safe".into());
        }
        if tool.confirmation {
            flags.push(if app.locked(tool) {
                "needs confirmation (press c)".into()
            } else {
                "confirmed for this session".into()
            });
        }
        let details = Text::from(vec![
            Line::from(tool.description.clone()),
            Line::from(Span::styled(
                flags.join(" · "),
                Style::new().fg(Color::DarkGray),
            )),
            Line::from(vec!["input:  ".bold(), Span::raw(tool.input.clone())]),
            Line::from(vec!["output: ".bold(), Span::raw(tool.output.clone())]),
        ]);
        frame.render_widget(
            Paragraph::new(details)
                .block(border(&tool.name, false))
                .wrap(Wrap { trim: true }),
            details_area,
        );
    }

    let arguments = app.arguments();
    draw_input(
        frame,
        "Arguments (JSON)",
        &arguments,
        args_area,
        app.focus == Focus::Arguments,
    );
    draw_input(
        frame,
        "request_id",
        &app.request_id(),
        id_area,
        app.focus == Focus::RequestId,
    );
    draw_result(frame, app, result);
}

fn draw_input(frame: &mut Frame, title: &str, input: &Input, area: Rect, focused: bool) {
    let inner = area.width.saturating_sub(2) as usize;
    // Keep the cursor visible when the text is wider than the field.
    let skip = input.cursor.saturating_sub(inner.saturating_sub(1));
    let visible: String = input.text.chars().skip(skip).take(inner).collect();
    frame.render_widget(Paragraph::new(visible).block(border(title, focused)), area);
    if focused {
        let x = area.x + 1 + (input.cursor - skip) as u16;
        frame.set_cursor_position((x.min(area.right().saturating_sub(2)), area.y + 1));
    }
}

fn draw_result(frame: &mut Frame, app: &App, area: Rect) {
    let focused = app.focus == Focus::Result;
    let Some(call) = &app.result else {
        let hint = "Pick a tool, edit its arguments and press Enter.";
        frame.render_widget(
            Paragraph::new(Span::styled(hint, Style::new().fg(Color::DarkGray)))
                .block(border("Result", focused)),
            area,
        );
        return;
    };
    let mut header = vec![
        Span::styled(call.status.clone(), status_style(&call.status)),
        Span::raw(format!(" · {} · {:.1} ms", call.tool, call.duration_ms)),
    ];
    if call.replayed {
        header.push(Span::raw(" · "));
        header.push(Span::styled(
            " replayed ",
            Style::new()
                .fg(Color::Black)
                .bg(Color::Magenta)
                .add_modifier(Modifier::BOLD),
        ));
    }
    let mut lines = vec![Line::from(header)];
    if let Some(code) = &call.error_code {
        let mut error = vec![Span::styled(
            code.clone(),
            Style::new().fg(Color::Red).bold(),
        )];
        if let Some(action) = &call.suggested_action {
            error.push(Span::raw(" · suggested action: "));
            error.push(Span::styled(
                action.clone(),
                Style::new().fg(Color::Yellow).bold(),
            ));
        }
        lines.push(Line::from(error));
    }
    lines.push(Line::raw(""));
    let pretty = serde_json::to_string_pretty(&call.body).unwrap_or_default();
    lines.extend(pretty.lines().map(|l| Line::raw(l.to_owned())));
    frame.render_widget(
        Paragraph::new(lines)
            .block(border("Result", focused))
            .scroll((app.scroll, 0)),
        area,
    );
}

fn centered(frame: &Frame, width: u16, height: u16) -> Rect {
    let area = frame.area();
    let width = width.min(area.width);
    let height = height.min(area.height);
    Rect::new(
        area.x + (area.width - width) / 2,
        area.y + (area.height - height) / 2,
        width,
        height,
    )
}

fn draw_help(frame: &mut Frame) {
    let area = centered(frame, 62, 17);
    let rows = [
        ("↑ ↓ / j k", "select a tool, or scroll the result"),
        (
            "Tab / Shift+Tab",
            "move focus: tools, arguments, request_id, result",
        ),
        (
            "Enter",
            "run the selected tool (from arguments or request_id)",
        ),
        ("Ctrl+R", "reset the arguments to the schema's example"),
        ("Ctrl+U", "clear the field being edited"),
        ("c", "grant or revoke confirm:<tool> for this session"),
        ("r", "run the last call again (shows idempotent replays)"),
        ("h", "history; Enter runs a past call again"),
        ("Esc", "leave a field, close a window, or quit"),
        ("q / Ctrl+C", "quit"),
    ];
    let mut lines: Vec<Line> = rows
        .iter()
        .map(|(key, what)| Line::from(vec![format!("{key:<16}").bold().cyan(), Span::raw(*what)]))
        .collect();
    lines.push(Line::raw(""));
    lines.push(Line::from(Span::styled(
        "Agents: speak carmy-console/1 with `carmy console --jsonl`.",
        Style::new().fg(Color::DarkGray),
    )));
    frame.render_widget(Clear, area);
    frame.render_widget(Paragraph::new(lines).block(border("Help", true)), area);
}

fn draw_confirm(frame: &mut Frame, tool: &str, grant: bool) {
    let area = centered(frame, 56, 6);
    let question = if grant {
        format!("Grant confirm:{tool} for this session?")
    } else {
        format!("Revoke confirm:{tool}?")
    };
    let text = Text::from(vec![
        Line::from(question.bold()),
        Line::raw(""),
        Line::from(vec![
            "y".green().bold(),
            Span::raw(" yes   "),
            "n".red().bold(),
            Span::raw(" no"),
        ]),
    ]);
    frame.render_widget(Clear, area);
    frame.render_widget(
        Paragraph::new(text).block(border("Confirmation", true)),
        area,
    );
}

fn draw_history(frame: &mut Frame, app: &App, selected: usize) {
    let area = centered(frame, 80, 16);
    let items: Vec<ListItem> = app
        .history
        .iter()
        .map(|call| {
            let mut spans = vec![
                Span::styled(format!("{:<10}", call.status), status_style(&call.status)),
                Span::raw(format!(" {} {}", call.tool, call.arguments)),
            ];
            if !call.request_id.is_empty() {
                spans.push(Span::styled(
                    format!("  request_id={}", call.request_id),
                    Style::new().fg(Color::DarkGray),
                ));
            }
            ListItem::new(Line::from(spans))
        })
        .collect();
    let mut state = ListState::default().with_selected(Some(selected));
    frame.render_widget(Clear, area);
    frame.render_stateful_widget(
        List::new(items)
            .block(border("History · Enter run again · Esc close", true))
            .highlight_style(Style::new().add_modifier(Modifier::REVERSED)),
        area,
        &mut state,
    );
}
