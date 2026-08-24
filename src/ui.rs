//! 描画。`App` を読むだけで、状態は変えない。
//! 例外はレイアウトから決まる `viewport`（ページ移動の幅）で、これだけ書き戻す。

use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, List, ListItem, ListState, Paragraph, Tabs, Wrap};
use ratatui::Frame;

use crate::app::{App, Focus, LineKind, Mode, View};
use crate::keys::HELP;
use crate::model::TaskStatus;

const ACCENT: Color = Color::Cyan;
const DIM: Color = Color::DarkGray;
const DONE: Color = Color::Green;
const PROGRESS: Color = Color::Yellow;

pub fn draw(app: &mut App, frame: &mut Frame) {
    let root = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Min(3),
            Constraint::Length(1),
        ])
        .split(frame.area());

    draw_tabs(app, frame, root[0]);
    match app.view {
        View::Notes => draw_notes(app, frame, root[1]),
        View::Todo => draw_todo(app, frame, root[1]),
        View::Projects => draw_projects(app, frame, root[1]),
        View::Search => draw_search(app, frame, root[1]),
    }
    draw_footer(app, frame, root[2]);

    if app.mode == Mode::Help {
        draw_help(app, frame, frame.area());
    }
}

fn draw_tabs(app: &App, frame: &mut Frame, area: Rect) {
    let selected = View::ALL.iter().position(|v| *v == app.view).unwrap_or(0);
    let titles: Vec<Line> = View::ALL.iter().map(|v| Line::from(v.title())).collect();
    let tabs = Tabs::new(titles)
        .select(selected)
        .style(Style::default().fg(DIM))
        .highlight_style(
            Style::default()
                .fg(ACCENT)
                .add_modifier(Modifier::BOLD | Modifier::REVERSED),
        )
        .divider(" ");
    frame.render_widget(tabs, area);
}

fn draw_notes(app: &mut App, frame: &mut Frame, area: Rect) {
    let cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Length(18), Constraint::Min(20)])
        .split(area);

    let visible = app.visible_notes();
    let items: Vec<ListItem> = app
        .notes
        .iter()
        .take(visible)
        .map(|note| {
            let count = note.entries.len();
            ListItem::new(Line::from(vec![
                Span::raw(note.date.clone()),
                Span::styled(format!(" {count}"), Style::default().fg(DIM)),
            ]))
        })
        .collect();

    let title = if visible < app.notes.len() {
        format!("日付 {visible}/{}", app.notes.len())
    } else {
        "日付".to_string()
    };
    let list = List::new(items)
        .block(bordered(&title, app.focus == Focus::List))
        .highlight_style(cursor_style(app.focus == Focus::List))
        .highlight_symbol("");
    let mut state = ListState::default();
    if !app.notes.is_empty() {
        state.select(Some(app.note_sel));
    }
    frame.render_stateful_widget(list, cols[0], &mut state);

    // 本文。ページ移動の幅を枠の内側の高さに合わせる。
    let inner_height = cols[1].height.saturating_sub(2) as usize;
    app.viewport = inner_height.max(1);

    let title = app
        .selected_note()
        .map(|n| n.date.clone())
        .unwrap_or_else(|| "ノートがありません".into());
    let lines: Vec<Line> = app
        .detail_lines()
        .into_iter()
        .map(|l| Line::from(Span::styled(l.text, style_for(l.kind))))
        .collect();
    let total = lines.len();
    let scroll = app.detail_scroll.min(total.saturating_sub(1)) as u16;
    let paragraph = Paragraph::new(lines)
        .block(bordered(&title, app.focus == Focus::Detail))
        .scroll((scroll, 0));
    frame.render_widget(paragraph, cols[1]);
}

fn draw_todo(app: &mut App, frame: &mut Frame, area: Rect) {
    app.viewport = area.height.saturating_sub(2).max(1) as usize;

    let items: Vec<ListItem> = app
        .todos
        .iter()
        .map(|(_, item)| {
            let (mark, color) = match item.status {
                TaskStatus::Backlog => ("[ ]", Color::Reset),
                TaskStatus::InProgress => ("[-]", PROGRESS),
                TaskStatus::Done => ("[x]", DONE),
            };
            let mut spans = vec![
                Span::styled(format!("{} ", item.date), Style::default().fg(DIM)),
                Span::styled(format!("{mark} "), Style::default().fg(color)),
            ];
            if !item.group.is_empty() {
                spans.push(Span::styled(
                    format!("{} / ", item.group),
                    Style::default().fg(ACCENT),
                ));
            }
            let title_style = if item.status == TaskStatus::Done {
                Style::default().fg(DIM).add_modifier(Modifier::CROSSED_OUT)
            } else {
                Style::default()
            };
            spans.push(Span::styled(item.title.clone(), title_style));
            ListItem::new(Line::from(spans))
        })
        .collect();

    let title = format!("ToDo  {} 件  Space で状態を進める", app.todos.len());
    let list = List::new(items)
        .block(bordered(&title, true))
        .highlight_style(cursor_style(true));
    let mut state = ListState::default();
    if !app.todos.is_empty() {
        state.select(Some(app.todo_sel));
    }
    frame.render_stateful_widget(list, area, &mut state);
}

fn draw_projects(app: &mut App, frame: &mut Frame, area: Rect) {
    let cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(35), Constraint::Min(20)])
        .split(area);
    app.viewport = cols[0].height.saturating_sub(2).max(1) as usize;

    let items: Vec<ListItem> = app
        .projects
        .iter()
        .map(|p| {
            let open = p.count(TaskStatus::Backlog) + p.count(TaskStatus::InProgress);
            let mut spans = vec![Span::raw(p.name.clone())];
            if p.is_archived() {
                spans.push(Span::styled(" (archived)", Style::default().fg(DIM)));
            }
            spans.push(Span::styled(
                format!("  未完 {open}"),
                Style::default().fg(if open > 0 { PROGRESS } else { DIM }),
            ));
            ListItem::new(Line::from(spans))
        })
        .collect();

    let list = List::new(items)
        .block(bordered("プロジェクト", true))
        .highlight_style(cursor_style(true));
    let mut state = ListState::default();
    if !app.projects.is_empty() {
        state.select(Some(app.project_sel));
    }
    frame.render_stateful_widget(list, cols[0], &mut state);

    let mut lines: Vec<Line> = Vec::new();
    let title = if let Some(project) = app.selected_project() {
        if !project.issue_url.is_empty() {
            lines.push(Line::from(Span::styled(
                project.issue_url.clone(),
                Style::default().fg(DIM),
            )));
            lines.push(Line::from(""));
        }
        for status in [
            TaskStatus::InProgress,
            TaskStatus::Backlog,
            TaskStatus::Done,
        ] {
            let tasks: Vec<_> = project
                .tasks
                .iter()
                .filter(|t| t.status == status)
                .collect();
            if tasks.is_empty() {
                continue;
            }
            lines.push(Line::from(Span::styled(
                format!("{} ({})", status.label(), tasks.len()),
                Style::default()
                    .fg(match status {
                        TaskStatus::Done => DONE,
                        TaskStatus::InProgress => PROGRESS,
                        TaskStatus::Backlog => ACCENT,
                    })
                    .add_modifier(Modifier::BOLD),
            )));
            for task in tasks {
                lines.push(Line::from(format!("  {} {}", status.marker(), task.title)));
            }
            lines.push(Line::from(""));
        }
        if lines.is_empty() {
            lines.push(Line::from(Span::styled(
                "タスクがありません",
                Style::default().fg(DIM),
            )));
        }
        project.name.clone()
    } else {
        "プロジェクトがありません".to_string()
    };

    frame.render_widget(
        Paragraph::new(lines)
            .block(bordered(&title, false))
            .wrap(Wrap { trim: false }),
        cols[1],
    );
}

fn draw_search(app: &mut App, frame: &mut Frame, area: Rect) {
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(3), Constraint::Min(3)])
        .split(area);
    app.viewport = rows[1].height.saturating_sub(2).max(1) as usize;

    let editing = app.mode == Mode::Search;
    let input = Paragraph::new(Line::from(vec![
        Span::styled("/ ", Style::default().fg(ACCENT)),
        Span::raw(app.query.clone()),
    ]))
    .block(bordered("検索", editing));
    frame.render_widget(input, rows[0]);

    if editing {
        // 入力欄の枠の内側にカーソルを置く。"/ " の 2 桁分ずらす。
        let x = rows[0].x + 3 + app.query.chars().count() as u16;
        frame.set_cursor_position((x.min(rows[0].right().saturating_sub(2)), rows[0].y + 1));
    }

    let items: Vec<ListItem> = app
        .hits
        .iter()
        .map(|hit| {
            ListItem::new(Line::from(vec![
                Span::styled(format!("{} ", hit.date), Style::default().fg(DIM)),
                Span::raw(hit.preview.clone()),
            ]))
        })
        .collect();

    let title = if app.query.trim().is_empty() {
        "語を入力すると全エントリを検索".to_string()
    } else {
        format!("{} 件  Enter で該当箇所へ", app.hits.len())
    };
    let list = List::new(items)
        .block(bordered(&title, !editing))
        .highlight_style(cursor_style(true));
    let mut state = ListState::default();
    if !app.hits.is_empty() {
        state.select(Some(app.search_sel));
    }
    frame.render_stateful_widget(list, rows[1], &mut state);
}

/// 画面下部の 1 行。入力中と確認待ちは、そこに出す。
fn draw_footer(app: &App, frame: &mut Frame, area: Rect) {
    if app.mode == Mode::Insert {
        if let Some(prompt) = &app.prompt {
            let line = Line::from(vec![
                Span::styled(
                    format!("{}: ", prompt.label()),
                    Style::default().fg(ACCENT).add_modifier(Modifier::BOLD),
                ),
                Span::raw(app.input.clone()),
            ]);
            frame.render_widget(Paragraph::new(line), area);
            let prefix = prompt.label().chars().count() + 2;
            let x = area.x + prefix as u16 + app.input.chars().count() as u16;
            frame.set_cursor_position((x.min(area.right().saturating_sub(1)), area.y));
            return;
        }
    }
    if app.mode == Mode::Confirm {
        if let Some(confirm) = &app.confirm {
            frame.render_widget(
                Paragraph::new(Line::from(Span::styled(
                    confirm.question(),
                    Style::default().fg(Color::Black).bg(PROGRESS),
                ))),
                area,
            );
            return;
        }
    }
    draw_status(app, frame, area);
}

fn draw_status(app: &App, frame: &mut Frame, area: Rect) {
    let line = match &app.status {
        Some(message) => Line::from(Span::styled(
            message.clone(),
            Style::default().fg(Color::Black).bg(ACCENT),
        )),
        None => Line::from(vec![
            Span::styled(app.summary(), Style::default().fg(DIM)),
            Span::styled("   ? ヘルプ   q 終了", Style::default().fg(DIM)),
        ]),
    };
    frame.render_widget(Paragraph::new(line), area);
}

fn draw_help(app: &App, frame: &mut Frame, area: Rect) {
    // キー一覧 + 空行 + データディレクトリ 2 行 + 閉じ方 + 枠。
    let height = (HELP.len() + 7).min(area.height as usize) as u16;
    let width = 76.min(area.width.saturating_sub(2));
    let popup = Rect {
        x: area.x + (area.width.saturating_sub(width)) / 2,
        y: area.y + (area.height.saturating_sub(height)) / 2,
        width,
        height,
    };

    let mut lines: Vec<Line> = HELP
        .iter()
        .map(|(key, desc)| {
            Line::from(vec![
                Span::styled(format!("{key:<18}"), Style::default().fg(ACCENT)),
                Span::raw(*desc),
            ])
        })
        .collect();
    lines.push(Line::from(""));
    lines.push(Line::from(vec![
        Span::styled(format!("{:<18}", "データ"), Style::default().fg(ACCENT)),
        Span::raw(app.config.data_dir.display().to_string()),
    ]));
    lines.push(Line::from(vec![
        Span::styled(
            format!("{:<18}", "設定の出どころ"),
            Style::default().fg(ACCENT),
        ),
        Span::raw(app.config.source.clone()),
    ]));
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "任意のキーで閉じる",
        Style::default().fg(DIM),
    )));

    frame.render_widget(Clear, popup);
    frame.render_widget(
        Paragraph::new(lines).block(bordered("キー操作", true)),
        popup,
    );
}

fn bordered(title: &str, focused: bool) -> Block<'static> {
    let style = if focused {
        Style::default().fg(ACCENT)
    } else {
        Style::default().fg(DIM)
    };
    Block::default()
        .borders(Borders::ALL)
        .border_style(style)
        .title(Span::styled(
            format!(" {title} "),
            if focused {
                Style::default().fg(ACCENT).add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(DIM)
            },
        ))
}

fn cursor_style(focused: bool) -> Style {
    if focused {
        Style::default()
            .bg(Color::DarkGray)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().bg(Color::Black)
    }
}

fn style_for(kind: LineKind) -> Style {
    match kind {
        LineKind::Header => Style::default().fg(ACCENT).add_modifier(Modifier::BOLD),
        LineKind::Heading => Style::default()
            .fg(Color::Magenta)
            .add_modifier(Modifier::BOLD),
        LineKind::ListItem => Style::default(),
        LineKind::Quote => Style::default().fg(DIM).add_modifier(Modifier::ITALIC),
        LineKind::Code => Style::default().fg(Color::LightGreen),
        LineKind::Body => Style::default(),
    }
}
