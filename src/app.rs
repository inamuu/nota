//! アプリ状態と更新ロジック。
//!
//! 状態は `App` 1 つに集約し、変更は必ず `Msg` を通す。描画は `App` を読むだけで、
//! 状態を書き換えない。編集機能を足すときは `Msg` を増やして `update` に分岐を
//! 加えるだけで済むようにしてある。

use anyhow::Result;

use crate::config::Config;
use crate::model::{DailyNote, Project, TaskStatus, TodoItem};
use crate::store::Store;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum View {
    Notes,
    Todo,
    Projects,
    Search,
}

impl View {
    pub const ALL: [View; 4] = [View::Notes, View::Todo, View::Projects, View::Search];

    pub fn title(self) -> &'static str {
        match self {
            Self::Notes => "1 ノート",
            Self::Todo => "2 ToDo",
            Self::Projects => "3 プロジェクト",
            Self::Search => "4 検索",
        }
    }
}

/// キー入力の解釈を切り替えるモード。挿入モードを足す前提で enum にしてある。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Normal,
    /// 検索クエリ入力中。
    Search,
    Help,
}

/// ビュー内のフォーカス。左の一覧か、右の本文か。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Focus {
    List,
    Detail,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Move {
    Up,
    Down,
    PageUp,
    PageDown,
    Top,
    Bottom,
}

#[derive(Debug, Clone)]
pub enum Msg {
    Quit,
    SwitchView(View),
    NextView,
    PrevView,
    Move(Move),
    ToggleFocus,
    /// ToDo のチェック状態を 1 段進める。
    CycleTodo,
    Reload,
    SearchStart,
    SearchInput(char),
    SearchBackspace,
    SearchClear,
    SearchCommit,
    SearchCancel,
    ToggleHelp,
    DismissStatus,
}

/// 本文ペインの 1 行。描画とスクロール位置の計算で共用する。
#[derive(Debug, Clone)]
pub struct DetailLine {
    pub text: String,
    pub kind: LineKind,
    /// この行が属するエントリ。ヘッダ行も含む。
    pub entry_idx: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LineKind {
    /// エントリの区切り（時刻とタグ）。
    Header,
    Heading,
    ListItem,
    Quote,
    Code,
    Body,
}

#[derive(Debug, Clone)]
pub struct SearchHit {
    pub note_idx: usize,
    pub entry_idx: usize,
    pub date: String,
    pub preview: String,
}

pub struct App {
    pub config: Config,
    store: Store,

    pub notes: Vec<DailyNote>,
    pub projects: Vec<Project>,
    /// ToDo 行と、それが属するノートの添字。書き戻しに添字が必要。
    pub todos: Vec<(usize, TodoItem)>,

    pub view: View,
    pub mode: Mode,
    pub focus: Focus,

    pub note_sel: usize,
    pub detail_scroll: usize,
    pub todo_sel: usize,
    pub project_sel: usize,
    pub search_sel: usize,

    pub query: String,
    pub hits: Vec<SearchHit>,

    pub status: Option<String>,
    pub should_quit: bool,
    /// 直近の描画で本文ペインに収まった行数。ページ移動の幅に使う。
    pub viewport: usize,
}

impl App {
    pub fn new(config: Config) -> Result<Self> {
        let store = Store::new(config.data_dir.clone());
        let notes = store.load_notes()?;
        let projects = store.load_projects()?;
        let mut app = Self {
            config,
            store,
            notes,
            projects,
            todos: Vec::new(),
            view: View::Notes,
            mode: Mode::Normal,
            focus: Focus::List,
            note_sel: 0,
            detail_scroll: 0,
            todo_sel: 0,
            project_sel: 0,
            search_sel: 0,
            query: String::new(),
            hits: Vec::new(),
            status: None,
            should_quit: false,
            viewport: 20,
        };
        app.rebuild_todos();
        Ok(app)
    }

    pub fn update(&mut self, msg: Msg) {
        // 何か操作したらメッセージは消す。DismissStatus 以外でも同様。
        if !matches!(msg, Msg::DismissStatus) {
            self.status = None;
        }
        match msg {
            Msg::Quit => self.should_quit = true,
            Msg::DismissStatus => self.status = None,
            Msg::ToggleHelp => {
                self.mode = if self.mode == Mode::Help {
                    Mode::Normal
                } else {
                    Mode::Help
                };
            }
            Msg::SwitchView(view) => self.switch_view(view),
            Msg::NextView => self.cycle_view(1),
            Msg::PrevView => self.cycle_view(-1),
            Msg::ToggleFocus => {
                self.focus = match self.focus {
                    Focus::List => Focus::Detail,
                    Focus::Detail => Focus::List,
                };
            }
            Msg::Move(m) => self.apply_move(m),
            Msg::CycleTodo => self.cycle_todo(),
            Msg::Reload => self.reload(),
            Msg::SearchStart => {
                self.view = View::Search;
                self.mode = Mode::Search;
                self.focus = Focus::List;
            }
            Msg::SearchInput(c) => {
                self.query.push(c);
                self.run_search();
            }
            Msg::SearchBackspace => {
                self.query.pop();
                self.run_search();
            }
            Msg::SearchClear => {
                self.query.clear();
                self.run_search();
            }
            Msg::SearchCommit => self.commit_search(),
            Msg::SearchCancel => {
                self.mode = Mode::Normal;
            }
        }
    }

    fn switch_view(&mut self, view: View) {
        self.view = view;
        self.focus = Focus::List;
        if view == View::Search {
            self.mode = Mode::Search;
        } else if self.mode == Mode::Search {
            self.mode = Mode::Normal;
        }
    }

    fn cycle_view(&mut self, delta: i32) {
        let cur = View::ALL.iter().position(|v| *v == self.view).unwrap_or(0) as i32;
        let len = View::ALL.len() as i32;
        let next = (cur + delta).rem_euclid(len) as usize;
        self.switch_view(View::ALL[next]);
    }

    fn apply_move(&mut self, m: Move) {
        // 本文ペインにフォーカスがあるときはスクロール。それ以外は一覧のカーソル。
        if self.view == View::Notes && self.focus == Focus::Detail {
            let max = self.detail_lines().len().saturating_sub(1);
            self.detail_scroll = shift(self.detail_scroll, m, max, self.viewport);
            return;
        }
        let (sel, len) = match self.view {
            View::Notes => (&mut self.note_sel, self.notes.len()),
            View::Todo => (&mut self.todo_sel, self.todos.len()),
            View::Projects => (&mut self.project_sel, self.projects.len()),
            View::Search => (&mut self.search_sel, self.hits.len()),
        };
        if len == 0 {
            *sel = 0;
            return;
        }
        let page = self.viewport;
        *sel = shift(*sel, m, len - 1, page);
        if self.view == View::Notes {
            self.detail_scroll = 0;
        }
    }

    fn cycle_todo(&mut self) {
        let Some((note_idx, item)) = self.todos.get(self.todo_sel).cloned() else {
            self.status = Some("ToDo がありません".into());
            return;
        };
        let next = item.status.next();
        let Some(note) = self.notes.get_mut(note_idx) else {
            return;
        };
        if !note.set_task_status(item.line, next) {
            self.status = Some("チェック欄が見つかりません".into());
            return;
        }
        if let Err(err) = self.store.save_note(&self.notes[note_idx]) {
            // 保存に失敗したら画面上の状態も戻す。ファイルと表示を食い違わせない。
            self.notes[note_idx].set_task_status(item.line, item.status);
            self.status = Some(format!("保存に失敗しました: {err}"));
            return;
        }
        self.rebuild_todos();
        self.status = Some(format!("{} → {}", item.title, next.label()));
    }

    fn reload(&mut self) {
        match (self.store.load_notes(), self.store.load_projects()) {
            (Ok(notes), Ok(projects)) => {
                self.notes = notes;
                self.projects = projects;
                self.clamp_all();
                self.rebuild_todos();
                self.run_search();
                self.status = Some(format!("再読み込みしました（{} 件）", self.notes.len()));
            }
            (Err(err), _) | (_, Err(err)) => {
                self.status = Some(format!("再読み込みに失敗しました: {err}"));
            }
        }
    }

    fn clamp_all(&mut self) {
        clamp(&mut self.note_sel, self.notes.len());
        clamp(&mut self.todo_sel, self.todos.len());
        clamp(&mut self.project_sel, self.projects.len());
        clamp(&mut self.search_sel, self.hits.len());
        self.detail_scroll = 0;
    }

    pub fn rebuild_todos(&mut self) {
        let mut out = Vec::new();
        for (idx, note) in self.notes.iter().enumerate() {
            for item in note.todo_items() {
                out.push((idx, item));
            }
        }
        self.todos = out;
        clamp(&mut self.todo_sel, self.todos.len());
    }

    fn run_search(&mut self) {
        self.search_sel = 0;
        self.hits.clear();
        let query = self.query.trim().to_lowercase();
        if query.is_empty() {
            return;
        }
        // 空白区切りの語をすべて含むエントリを拾う。
        let terms: Vec<&str> = query.split_whitespace().collect();
        for (note_idx, note) in self.notes.iter().enumerate() {
            for (entry_idx, entry) in note.entries.iter().enumerate() {
                let mut haystack = note.body_of(entry).join("\n").to_lowercase();
                haystack.push('\n');
                haystack.push_str(&entry.tags.join(",").to_lowercase());
                if !terms.iter().all(|t| haystack.contains(t)) {
                    continue;
                }
                self.hits.push(SearchHit {
                    note_idx,
                    entry_idx,
                    date: note.date.clone(),
                    preview: preview_line(note, entry_idx, &terms),
                });
                if self.hits.len() >= 500 {
                    return;
                }
            }
        }
    }

    fn commit_search(&mut self) {
        let Some(hit) = self.hits.get(self.search_sel).cloned() else {
            self.mode = Mode::Normal;
            return;
        };
        self.view = View::Notes;
        self.mode = Mode::Normal;
        self.focus = Focus::Detail;
        self.note_sel = hit.note_idx;
        self.detail_scroll = self.entry_offset(hit.note_idx, hit.entry_idx);
    }

    /// 本文ペインの何行目にそのエントリが現れるか。検索からのジャンプで使う。
    fn entry_offset(&self, note_idx: usize, entry_idx: usize) -> usize {
        let lines = self.detail_lines_of(note_idx);
        lines
            .iter()
            .position(|l| l.entry_idx == entry_idx)
            .unwrap_or(0)
    }

    pub fn selected_note(&self) -> Option<&DailyNote> {
        self.notes.get(self.note_sel)
    }

    pub fn detail_lines(&self) -> Vec<DetailLine> {
        self.detail_lines_of(self.note_sel)
    }

    fn detail_lines_of(&self, note_idx: usize) -> Vec<DetailLine> {
        let Some(note) = self.notes.get(note_idx) else {
            return Vec::new();
        };
        let mut out = Vec::new();
        for (entry_idx, entry) in note.entries.iter().enumerate() {
            let tags = if entry.tags.is_empty() {
                String::new()
            } else {
                format!("  [{}]", entry.tags.join(", "))
            };
            out.push(DetailLine {
                text: format!("{}{}", short_time(&entry.created), tags),
                kind: LineKind::Header,
                entry_idx,
            });
            let mut in_code = false;
            for line in note.body_of(entry) {
                let trimmed = line.trim_start();
                if trimmed.starts_with("```") {
                    in_code = !in_code;
                    out.push(DetailLine {
                        text: line.to_string(),
                        kind: LineKind::Code,
                        entry_idx,
                    });
                    continue;
                }
                let kind = if in_code {
                    LineKind::Code
                } else if trimmed.starts_with('#') {
                    LineKind::Heading
                } else if trimmed.starts_with("- ") || trimmed.starts_with("* ") {
                    LineKind::ListItem
                } else if trimmed.starts_with('>') {
                    LineKind::Quote
                } else {
                    LineKind::Body
                };
                out.push(DetailLine {
                    text: line.to_string(),
                    kind,
                    entry_idx,
                });
            }
            out.push(DetailLine {
                text: String::new(),
                kind: LineKind::Body,
                entry_idx,
            });
        }
        out
    }

    pub fn selected_project(&self) -> Option<&Project> {
        self.projects.get(self.project_sel)
    }

    /// 画面下部に出す集計。
    pub fn summary(&self) -> String {
        let open = self
            .todos
            .iter()
            .filter(|(_, t)| t.status != TaskStatus::Done)
            .count();
        let active = self.projects.iter().filter(|p| !p.is_archived()).count();
        format!(
            "ノート {} / ToDo 未完 {} / プロジェクト {}",
            self.notes.len(),
            open,
            active
        )
    }
}

fn clamp(sel: &mut usize, len: usize) {
    if len == 0 {
        *sel = 0;
    } else if *sel >= len {
        *sel = len - 1;
    }
}

fn shift(current: usize, m: Move, max: usize, page: usize) -> usize {
    let page = page.max(1);
    match m {
        Move::Up => current.saturating_sub(1),
        Move::Down => (current + 1).min(max),
        Move::PageUp => current.saturating_sub(page),
        Move::PageDown => (current + page).min(max),
        Move::Top => 0,
        Move::Bottom => max,
    }
}

/// `2026-02-26 11:25` から時刻部分だけ取る。取れなければそのまま返す。
fn short_time(created: &str) -> String {
    created
        .split_once(' ')
        .map(|(_, time)| time.to_string())
        .unwrap_or_else(|| created.to_string())
}

/// 検索語を含む最初の行を抜き出す。無ければ本文の先頭行。
fn preview_line(note: &DailyNote, entry_idx: usize, terms: &[&str]) -> String {
    let entry = &note.entries[entry_idx];
    let body = note.body_of(entry);
    let hit = body
        .iter()
        .find(|line| {
            let lower = line.to_lowercase();
            terms.iter().any(|t| lower.contains(t))
        })
        .or_else(|| body.iter().find(|l| !l.trim().is_empty()))
        .copied()
        .unwrap_or("");
    let cleaned = hit.trim().trim_start_matches('#').trim();
    cleaned.chars().take(120).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn moves_within_bounds() {
        assert_eq!(shift(0, Move::Up, 9, 5), 0);
        assert_eq!(shift(9, Move::Down, 9, 5), 9);
        assert_eq!(shift(0, Move::Bottom, 9, 5), 9);
        assert_eq!(shift(9, Move::Top, 9, 5), 0);
        assert_eq!(shift(7, Move::PageUp, 9, 5), 2);
        assert_eq!(shift(7, Move::PageDown, 9, 5), 9);
    }

    #[test]
    fn page_move_survives_zero_viewport() {
        assert_eq!(shift(3, Move::PageDown, 9, 0), 4);
    }

    #[test]
    fn clamps_selection_to_length() {
        let mut sel = 5;
        clamp(&mut sel, 3);
        assert_eq!(sel, 2);
        clamp(&mut sel, 0);
        assert_eq!(sel, 0);
    }

    #[test]
    fn extracts_time_only() {
        assert_eq!(short_time("2026-02-26 11:25"), "11:25");
        assert_eq!(short_time("2026-02-26"), "2026-02-26");
    }
}
