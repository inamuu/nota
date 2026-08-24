//! 描画。`App` を読むだけで、状態は変えない。
//! 例外はレイアウトから決まる `viewport`（ページ移動の幅）で、これだけ書き戻す。

use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, List, ListItem, ListState, Paragraph, Wrap};
use ratatui::Frame;

use crate::app::{App, Focus, LineKind, Mode, View};
use crate::keys::HELP;
use crate::model::TaskStatus;

const ACCENT: Color = Color::Cyan;
const DIM: Color = Color::DarkGray;
const DONE: Color = Color::Green;
const PROGRESS: Color = Color::Yellow;
/// 選択行の左に出すマーカー。行頭が揃うので目で追いやすい。
const CURSOR: &str = "▌ ";

/// 起動時のロゴ。
const LOGO: [&str; 6] = [
    "███╗   ██╗  ██████╗  ████████╗  █████╗ ",
    "████╗  ██║ ██╔═══██╗ ╚══██╔══╝ ██╔══██╗",
    "██╔██╗ ██║ ██║   ██║    ██║    ███████║",
    "██║╚██╗██║ ██║   ██║    ██║    ██╔══██║",
    "██║ ╚████║ ╚██████╔╝    ██║    ██║  ██║",
    "╚═╝  ╚═══╝  ╚═════╝     ╚═╝    ╚═╝  ╚═╝",
];

pub fn draw(app: &mut App, frame: &mut Frame) {
    if app.splash_visible() {
        draw_splash(app, frame, frame.area());
        return;
    }

    let root = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
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

/// 起動直後のロゴ。読み込んだ件数も出して、どのデータを開いたか分かるようにする。
fn draw_splash(app: &App, frame: &mut Frame, area: Rect) {
    let mut lines: Vec<Line> = Vec::new();
    // 縦位置を中央に寄せる。ロゴ 6 行 + 説明 5 行を目安にする。
    let content = LOGO.len() + 6;
    for _ in 0..area.height.saturating_sub(content as u16) / 2 {
        lines.push(Line::from(""));
    }
    for row in LOGO {
        lines.push(Line::from(Span::styled(
            row,
            Style::default().fg(ACCENT).add_modifier(Modifier::BOLD),
        )));
    }
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        format!(
            "Acta のデータをターミナルから  v{}",
            env!("CARGO_PKG_VERSION")
        ),
        Style::default().fg(DIM),
    )));
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        app.summary(),
        Style::default().fg(Color::Reset),
    )));
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "? キー操作   何かキーを押して開始",
        Style::default().fg(DIM),
    )));

    frame.render_widget(
        Paragraph::new(lines).alignment(ratatui::layout::Alignment::Center),
        area,
    );
}

fn draw_tabs(app: &App, frame: &mut Frame, area: Rect) {
    // 他のペインと同じ枠に入れる。
    let block = bordered("Menu", false);
    let inner = block.inner(area);
    frame.render_widget(block, area);
    if inner.height == 0 {
        return;
    }

    // タブは自前で組む。選択中を塗って、他は沈める。
    let mut spans = vec![Span::raw(" ")];
    for view in View::ALL {
        let selected = view == app.view;
        spans.push(Span::styled(
            format!(" {} ", view.title()),
            if selected {
                Style::default()
                    .fg(Color::Black)
                    .bg(ACCENT)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(DIM)
            },
        ));
        spans.push(Span::raw(" "));
    }
    // 右端にモードを出す。今どの状態かが常に見える。
    // 文字数ではなく領域を分けて右寄せする。全角が混ざると桁数と表示幅がずれる。
    let mode = match app.mode {
        Mode::Normal => None,
        Mode::Search => Some(("SEARCH", ACCENT)),
        Mode::Confirm => Some(("CONFIRM", PROGRESS)),
        Mode::Help => Some(("HELP", DONE)),
    };
    let Some((label, color)) = mode else {
        frame.render_widget(Paragraph::new(Line::from(spans)), inner);
        return;
    };

    let badge = format!(" {label} ");
    let width = badge.chars().count() as u16;
    let cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Min(0), Constraint::Length(width)])
        .split(inner);
    frame.render_widget(Paragraph::new(Line::from(spans)), cols[0]);
    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(
            badge,
            Style::default()
                .fg(Color::Black)
                .bg(color)
                .add_modifier(Modifier::BOLD),
        ))),
        cols[1],
    );
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
                Span::styled(format!("  {count}"), Style::default().fg(DIM)),
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
        .highlight_symbol(CURSOR);
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
    // エントリの区切りはペイン幅まで伸ばす。境界がひと目で分かる。
    let inner_width = cols[1].width.saturating_sub(2) as usize;
    let lines: Vec<Line> = app
        .detail_lines()
        .into_iter()
        .map(|l| {
            let span = Span::styled(l.text, style_for(l.kind));
            if l.kind != LineKind::Header {
                return Line::from(span);
            }
            let rest = inner_width.saturating_sub(span.width() + 1);
            Line::from(vec![
                span,
                Span::styled(format!(" {}", "─".repeat(rest)), Style::default().fg(DIM)),
            ])
        })
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
        .highlight_style(cursor_style(true))
        .highlight_symbol(CURSOR);
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
        .highlight_style(cursor_style(true))
        .highlight_symbol(CURSOR);
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
        .highlight_style(cursor_style(true))
        .highlight_symbol(CURSOR);
    let mut state = ListState::default();
    if !app.hits.is_empty() {
        state.select(Some(app.search_sel));
    }
    frame.render_stateful_widget(list, rows[1], &mut state);
}

/// 画面下部の 1 行。入力中と確認待ちは、そこに出す。
fn draw_footer(app: &App, frame: &mut Frame, area: Rect) {
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
        Style::default().fg(ACCENT).add_modifier(Modifier::BOLD)
    } else {
        Style::default().add_modifier(Modifier::BOLD)
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
