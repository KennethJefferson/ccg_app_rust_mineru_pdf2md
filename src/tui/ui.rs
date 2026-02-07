use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Gauge, Paragraph},
    Frame,
};

use crate::types::AppState;

use super::widgets;

const GREEN: Color = Color::Rgb(80, 200, 120);
const DARK_GREEN: Color = Color::Rgb(30, 70, 40);
const DIM: Color = Color::Rgb(100, 100, 100);

const MAX_FILES_VISIBLE: usize = 25;

pub fn render(frame: &mut Frame, state: &AppState, tick: usize) {
    let area = frame.area();

    let outer_block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(GREEN))
        .title(Line::from(vec![Span::styled(
            " MinerU PDF to Markdown ",
            Style::default()
                .fg(GREEN)
                .add_modifier(Modifier::BOLD),
        )]))
        .title_bottom(Line::from(vec![Span::styled(
            if state.shutdown_requested {
                " Shutting down... Press Ctrl+C again to force "
            } else {
                " [Ctrl+C to stop] "
            },
            Style::default().fg(DIM),
        )]));

    let inner = outer_block.inner(area);
    frame.render_widget(outer_block, area);

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .margin(1)
        .constraints([
            Constraint::Length(state.workers.len() as u16 + 2),
            Constraint::Length(3),
            Constraint::Min(5),
            Constraint::Length(2),
        ])
        .split(inner);

    render_workers(frame, chunks[0], state, tick);
    render_progress(frame, chunks[1], state);
    render_files(frame, chunks[2], state);
    render_stats(frame, chunks[3], state);
}

fn render_workers(frame: &mut Frame, area: Rect, state: &AppState, tick: usize) {
    let mut lines = vec![Line::from(Span::styled(
        " Workers",
        Style::default()
            .fg(GREEN)
            .add_modifier(Modifier::BOLD),
    ))];

    for (i, status) in state.workers.iter().enumerate() {
        lines.push(widgets::worker_line(i, status, tick));
    }

    let widget = Paragraph::new(lines);
    frame.render_widget(widget, area);
}

fn render_progress(frame: &mut Frame, area: Rect, state: &AppState) {
    let stats = &state.stats;
    let processed = stats.processed();
    let total = stats.queue_total();
    let ratio = if total == 0 {
        1.0
    } else {
        processed as f64 / total as f64
    };
    let pct = (ratio * 100.0) as u16;

    let label = format!("{processed}/{total}          {pct}%");

    let gauge = Gauge::default()
        .block(
            Block::default()
                .title(Span::styled(" Progress", Style::default().fg(GREEN)))
                .borders(Borders::NONE),
        )
        .gauge_style(Style::default().fg(GREEN).bg(DARK_GREEN))
        .ratio(ratio.min(1.0))
        .label(label);

    frame.render_widget(gauge, area);
}

fn render_files(frame: &mut Frame, area: Rect, state: &AppState) {
    let mut sorted_files: Vec<_> = state.files.iter().collect();
    sorted_files.sort_by_key(|f| widgets::file_sort_key(&f.status));

    let visible: Vec<_> = sorted_files
        .iter()
        .take(MAX_FILES_VISIBLE)
        .map(|f| widgets::file_line(&f.filename, &f.status))
        .collect();

    let mut lines = vec![Line::from(Span::styled(
        " Files",
        Style::default()
            .fg(GREEN)
            .add_modifier(Modifier::BOLD),
    ))];
    lines.extend(visible);

    let remaining = sorted_files.len().saturating_sub(MAX_FILES_VISIBLE);
    if remaining > 0 {
        lines.push(Line::from(Span::styled(
            format!("  ... and {remaining} more"),
            Style::default().fg(DIM),
        )));
    }

    let widget = Paragraph::new(lines);
    frame.render_widget(widget, area);
}

fn render_stats(frame: &mut Frame, area: Rect, state: &AppState) {
    let stats = &state.stats;
    let elapsed = widgets::format_duration(stats.elapsed().as_secs());

    let line = Line::from(vec![
        Span::styled(
            format!(" Completed: {}", stats.completed),
            Style::default().fg(GREEN),
        ),
        Span::styled("  ", Style::default()),
        Span::styled(
            format!("Failed: {}", stats.failed),
            Style::default().fg(if stats.failed > 0 {
                Color::Rgb(220, 80, 80)
            } else {
                DIM
            }),
        ),
        Span::styled("  ", Style::default()),
        Span::styled(
            format!("Skipped: {}", stats.skipped),
            Style::default().fg(if stats.skipped > 0 {
                Color::Rgb(220, 200, 80)
            } else {
                DIM
            }),
        ),
        Span::styled("  ", Style::default()),
        Span::styled(format!("Elapsed: {elapsed}"), Style::default().fg(DIM)),
    ]);

    let widget = Paragraph::new(vec![line]);
    frame.render_widget(widget, area);
}
