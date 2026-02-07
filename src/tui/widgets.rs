use ratatui::{
    style::{Color, Style},
    text::{Line, Span},
};

use crate::types::{FileStatus, WorkerStatus};

const BRAILLE_SPINNER: &[char] = &['⠋', '⠙', '⠹', '⠸', '⠼', '⠴', '⠦', '⠧', '⠇', '⠏'];
const GREEN: Color = Color::Rgb(80, 200, 120);
const DIM_GREEN: Color = Color::Rgb(40, 100, 60);
const RED: Color = Color::Rgb(220, 80, 80);
const YELLOW: Color = Color::Rgb(220, 200, 80);
const DIM: Color = Color::Rgb(100, 100, 100);

pub fn spinner_char(frame: usize) -> char {
    BRAILLE_SPINNER[frame % BRAILLE_SPINNER.len()]
}

pub fn worker_line(id: usize, status: &WorkerStatus, frame: usize) -> Line<'static> {
    match status {
        WorkerStatus::Idle => Line::from(vec![
            Span::styled(format!("  - Worker {id}: "), Style::default().fg(DIM)),
            Span::styled("idle", Style::default().fg(DIM)),
        ]),
        WorkerStatus::Processing { filename, started } => {
            let elapsed = started.elapsed().as_secs();
            let spin = spinner_char(frame);
            Line::from(vec![
                Span::styled(
                    format!("  {spin} Worker {id}: "),
                    Style::default().fg(GREEN),
                ),
                Span::styled(filename.clone(), Style::default().fg(Color::White)),
                Span::styled(format!("  ({elapsed}s)"), Style::default().fg(DIM)),
            ])
        }
        WorkerStatus::Done { filename, duration } => {
            let secs = duration.as_secs_f32();
            Line::from(vec![
                Span::styled(
                    format!("  ✓ Worker {id}: "),
                    Style::default().fg(DIM_GREEN),
                ),
                Span::styled(filename.clone(), Style::default().fg(DIM_GREEN)),
                Span::styled(format!("  ({secs:.1}s)"), Style::default().fg(DIM)),
            ])
        }
    }
}

pub fn file_line(filename: &str, status: &FileStatus) -> Line<'static> {
    match status {
        FileStatus::Pending => Line::from(vec![
            Span::styled("  - ", Style::default().fg(DIM)),
            Span::styled(filename.to_string(), Style::default().fg(DIM)),
        ]),
        FileStatus::Processing => Line::from(vec![
            Span::styled("  ⠹ ", Style::default().fg(GREEN)),
            Span::styled(filename.to_string(), Style::default().fg(Color::White)),
        ]),
        FileStatus::Completed { duration } => {
            let secs = duration.as_secs_f32();
            Line::from(vec![
                Span::styled("  ✓ ", Style::default().fg(GREEN)),
                Span::styled(filename.to_string(), Style::default().fg(GREEN)),
                Span::styled(format!("  {secs:.1}s"), Style::default().fg(DIM)),
            ])
        }
        FileStatus::Failed { error, duration } => {
            let secs = duration.as_secs_f32();
            Line::from(vec![
                Span::styled("  ✗ ", Style::default().fg(RED)),
                Span::styled(filename.to_string(), Style::default().fg(RED)),
                Span::styled(format!("  {error}"), Style::default().fg(YELLOW)),
                Span::styled(format!("  {secs:.1}s"), Style::default().fg(DIM)),
            ])
        }
        FileStatus::Skipped => Line::from(vec![
            Span::styled("  - ", Style::default().fg(DIM)),
            Span::styled(filename.to_string(), Style::default().fg(DIM)),
            Span::styled("  skipped", Style::default().fg(YELLOW)),
        ]),
    }
}

pub fn format_duration(secs: u64) -> String {
    if secs < 60 {
        format!("{secs}s")
    } else {
        let m = secs / 60;
        let s = secs % 60;
        format!("{m}m {s:02}s")
    }
}

pub fn file_sort_key(status: &FileStatus) -> u8 {
    match status {
        FileStatus::Processing => 0,
        FileStatus::Failed { .. } => 1,
        FileStatus::Completed { .. } => 2,
        FileStatus::Pending => 3,
        FileStatus::Skipped => 4,
    }
}
