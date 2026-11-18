use super::{ACCENT, DIM};
use crate::app::{App, Section, SortKey};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
pub(super) fn draw_header(f: &mut Frame, area: Rect, app: &App) {
    let mut spans = vec![
        Span::styled(
            " daffy ",
            Style::default().fg(Color::Black).bg(ACCENT).bold(),
        ),
        Span::raw("  "),
    ];
    for (key, sec) in [
        ("v", Section::Videos),
        ("d", Section::Folders),
        ("c", Section::Collections),
    ] {
        let active = app.section == sec;
        let style = if active {
            Style::default().fg(ACCENT).bold().underlined()
        } else {
            Style::default().fg(DIM)
        };
        spans.push(Span::styled(format!("[{key}] {}", sec.title()), style));
        spans.push(Span::raw("   "));
    }
    let count = match app.section {
        Section::Videos => app.view_videos.len(),
        Section::Folders => app.view_folders.len(),
        Section::Collections => app.view_collections.len(),
    };
    spans.push(Span::styled(format!("{count}"), Style::default().fg(DIM)));
    if !app.search.is_empty() {
        spans.push(Span::styled(
            format!("   / \"{}\"", app.search),
            Style::default().fg(ACCENT),
        ));
    }
    if app.sort != SortKey::Folder {
        spans.push(Span::styled(
            format!("   sort: {}", app.sort.label().to_lowercase()),
            Style::default().fg(ACCENT),
        ));
    }
    let left = Line::from(spans);
    f.render_widget(Paragraph::new(left.clone()), area);
    let right = format!("video: {} ", app.renderer.label());
    let rw = right.chars().count() as u16;
    if area.width > left.width() as u16 + rw + 2 {
        f.render_widget(
            Paragraph::new(Span::styled(right, Style::default().fg(DIM))),
            Rect {
                x: area.x + area.width - rw,
                y: area.y,
                width: rw,
                height: 1,
            },
        );
    }
}
