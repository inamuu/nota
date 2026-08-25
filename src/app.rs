//! アプリ状態と更新ロジック。
//!
//! 状態は `App` 1 つに集約し、変更は必ず `Msg` を通す。描画は `App` を読むだけで、
//! 状態を書き換えない。編集機能を足すときは `Msg` を増やして `update` に分岐を
//! 加えるだけで済むようにしてある。

use std::time::{Duration, Instant};

use anyhow::Result;

use crate::config::Config;
use crate::editor::{EditRequest, EditTarget};
use crate::model::{format_entry_block, DailyNote, Project, TaskStatus, TodoItem};
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
    /// y / n の確認待ち。
    Confirm,
    Help,
}

/// 確認待ちの操作。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Confirm {
    DeleteEntry { note_idx: usize, entry_idx: usize },
}

impl Confirm {
    pub fn question(&self) -> String {
        match self {
            Self::DeleteEntry { .. } => "このエントリを削除しますか？ y / n".to_string(),
        }
    }
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
    /// ノート一覧を直近だけにするか、全件にするか。
    ToggleAllNotes,
    /// アーカイブ済みのプロジェクトを出すかどうか。
    ToggleArchived,
    /// 選択中のエントリを $EDITOR で編集する。
    EditEntry,
    /// 今日のノートに新しいエントリを作る。
    NewEntry,
    /// 今日の ToDo を開く。無ければ進行中のタスクから作る。
    TodayTodo,
    /// 削除の確認を始める。
    DeleteEntry,
    ConfirmYes,
    ConfirmNo,
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
    /// 検索を開く前のビュー。Esc で戻る先。
    return_view: View,
    pub mode: Mode,
    pub focus: Focus,

    pub note_sel: usize,
    pub detail_scroll: usize,
    pub todo_sel: usize,
    pub project_sel: usize,
    /// プロジェクトのタスク一覧のカーソル。
    pub task_sel: usize,
    pub search_sel: usize,

    pub query: String,
    pub hits: Vec<SearchHit>,

    /// 確認待ちの操作。Mode::Confirm のときだけ入っている。
    pub confirm: Option<Confirm>,
    /// $EDITOR を開いてほしいという要求。端末を触るのはイベントループの仕事なので、
    /// ここに置いて main に取り出させる。
    pending_edit: Option<EditRequest>,

    /// 起動直後のロゴを消す時刻。過ぎたら、あるいはキーを押したら消える。
    splash: Option<Instant>,

    pub status: Option<String>,
    /// ノート一覧を全件出しているか。既定は直近だけ。
    pub show_all_notes: bool,
    /// アーカイブ済みのプロジェクトを出しているか。既定は隠す。
    pub show_archived: bool,
    pub should_quit: bool,
    /// 直近の描画で本文ペインに収まった行数。ページ移動の幅に使う。
    pub viewport: usize,
}

/// ロゴを出しておく時間。読み込みが速いので、少しだけ見えるようにする。
const SPLASH_DURATION: Duration = Duration::from_millis(1400);

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
            return_view: View::Notes,
            mode: Mode::Normal,
            focus: Focus::List,
            note_sel: 0,
            detail_scroll: 0,
            todo_sel: 0,
            project_sel: 0,
            task_sel: 0,
            search_sel: 0,
            query: String::new(),
            hits: Vec::new(),
            confirm: None,
            pending_edit: None,
            splash: Some(Instant::now() + SPLASH_DURATION),
            status: None,
            show_all_notes: false,
            show_archived: false,
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
            Msg::CycleTodo => {
                if self.view == View::Projects {
                    self.cycle_project_task();
                } else {
                    self.cycle_todo();
                }
            }
            Msg::Reload => self.reload(),
            Msg::ToggleArchived => {
                self.show_archived = !self.show_archived;
                let len = self.visible_projects().len();
                if self.project_sel >= len {
                    self.project_sel = len.saturating_sub(1);
                }
                self.task_sel = 0;
                self.status = Some(if self.show_archived {
                    format!("アーカイブ済みも表示（{} 件）", self.projects.len())
                } else {
                    format!("アーカイブ済みを除外（{} 件）", len)
                });
            }
            Msg::ToggleAllNotes => {
                self.show_all_notes = !self.show_all_notes;
                // 絞り込みで選択が範囲外になることがある。
                let len = self.visible_notes();
                if self.note_sel >= len {
                    self.note_sel = len.saturating_sub(1);
                    self.detail_scroll = 0;
                }
                self.status = Some(if self.show_all_notes {
                    format!("全 {} 件を表示", self.notes.len())
                } else {
                    format!("直近 {} 件を表示", self.visible_notes())
                });
            }
            Msg::SearchStart => self.switch_view(View::Search),
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
            // 検索は Esc 一発で抜けて、開く前のビューに戻る。
            Msg::SearchCancel => {
                self.mode = Mode::Normal;
                self.view = self.return_view;
                self.focus = Focus::List;
            }

            Msg::EditEntry => self.start_edit_entry(),
            Msg::NewEntry => self.start_new_entry(),
            Msg::TodayTodo => self.start_today_todo(),
            Msg::DeleteEntry => self.start_delete_entry(),
            Msg::ConfirmYes => self.apply_confirm(),
            Msg::ConfirmNo => {
                self.confirm = None;
                self.mode = Mode::Normal;
            }
        }
    }

    /// 起動直後のロゴを出す時間帯か。
    pub fn splash_visible(&self) -> bool {
        self.splash.is_some_and(|until| Instant::now() < until)
    }

    /// ロゴを閉じる。キーが押されたらすぐ消す。
    pub fn dismiss_splash(&mut self) {
        self.splash = None;
    }

    /// 状態行にメッセージを出す。イベントループからも使う。
    pub fn report(&mut self, message: String) {
        self.status = Some(message);
    }

    /// main が取り出してエディタを起動する。取り出したら要求は消える。
    pub fn take_edit_request(&mut self) -> Option<EditRequest> {
        self.pending_edit.take()
    }

    /// エディタから戻ってきた内容を反映する。
    pub fn apply_edit(&mut self, target: EditTarget, edited: Option<String>) {
        let Some(text) = edited else {
            self.status = Some("変更はありません".into());
            return;
        };
        match target {
            EditTarget::EntryBody {
                note_idx,
                entry_idx,
            } => self.apply_entry_body(note_idx, entry_idx, &text),
            EditTarget::ProjectTasks { project_idx } => {
                self.apply_project_tasks(project_idx, &text)
            }
            EditTarget::NewEntry => {
                let (tags, body) = crate::editor::decompose(&text);
                if body.trim().is_empty() {
                    self.status = Some("本文が空なので作成しませんでした".into());
                    return;
                }
                self.create_entry(&body, &tags);
            }
        }
    }

    fn apply_entry_body(&mut self, note_idx: usize, entry_idx: usize, text: &str) {
        let (tags, body) = crate::editor::decompose(text);
        if body.trim().is_empty() {
            self.status = Some("本文が空のままなので変更しませんでした".into());
            return;
        }
        let Some(note) = self.notes.get_mut(note_idx) else {
            return;
        };
        let before = note.to_text();
        if !note.replace_entry_body(entry_idx, &body) {
            self.status = Some("エントリが見つかりません".into());
            return;
        }
        // タグも 1 行目で編集できるので、変わっていれば反映する。
        if note.entries[entry_idx].tags != tags {
            note.set_entry_tags(entry_idx, &tags);
        }
        self.persist(note_idx, before, "保存しました");
    }

    /// 保存に失敗したら画面の状態も元に戻す。ファイルと表示を食い違わせない。
    fn persist(&mut self, note_idx: usize, before: String, success: &str) {
        let note = &self.notes[note_idx];
        let result = if note.path.is_file() {
            self.store.save_note(note)
        } else {
            self.store.save_new_note(note)
        };
        match result {
            Ok(()) => {
                self.rebuild_todos();
                self.run_search();
                self.status = Some(success.to_string());
            }
            Err(err) => {
                let note = &mut self.notes[note_idx];
                let path = note.path.clone();
                let date = note.date.clone();
                *note = DailyNote::parse(date, path, &before);
                self.rebuild_todos();
                self.status = Some(format!("保存に失敗しました: {err}"));
            }
        }
    }

    fn start_edit_entry(&mut self) {
        // プロジェクトビューではタスク一覧をまとめて編集する。
        if self.view == View::Projects {
            let visible = self.visible_projects();
            let Some(project_idx) = visible.get(self.project_sel).copied() else {
                self.status = Some("プロジェクトがありません".into());
                return;
            };
            self.pending_edit = Some(EditRequest {
                target: EditTarget::ProjectTasks { project_idx },
                initial: self.projects[project_idx].tasks_as_checklist(),
            });
            return;
        }
        let Some((note_idx, entry_idx)) = self.focused_entry() else {
            self.status = Some("編集するエントリがありません".into());
            return;
        };
        let note = &self.notes[note_idx];
        let entry = &note.entries[entry_idx];
        let initial = crate::editor::compose(&entry.tags, &note.body_of(entry).join("\n"));
        self.pending_edit = Some(EditRequest {
            target: EditTarget::EntryBody {
                note_idx,
                entry_idx,
            },
            initial,
        });
    }

    /// チェックリストの編集結果を project.json に書く。
    fn apply_project_tasks(&mut self, project_idx: usize, text: &str) {
        let tasks = crate::model::parse_checklist(text);
        let Some(project) = self.projects.get(project_idx) else {
            return;
        };
        // 全部消して保存されたときは、事故を疑って何もしない。
        if tasks.is_empty() && !project.tasks.is_empty() {
            self.status = Some("タスクが空になったので変更しませんでした".into());
            return;
        }
        let now = chrono::Local::now().timestamp_millis();
        if let Err(err) = self.store.save_project_tasks(project, &tasks, now) {
            self.status = Some(format!("保存に失敗しました: {err}"));
            return;
        }
        self.reload_projects();
        self.status = Some(format!("タスクを保存しました（{} 件）", tasks.len()));
    }

    /// プロジェクトだけ読み直す。ノートまで読み直す必要はない。
    fn reload_projects(&mut self) {
        match self.store.load_projects() {
            Ok(projects) => {
                self.projects = projects;
                let len = self.visible_projects().len();
                if self.project_sel >= len {
                    self.project_sel = len.saturating_sub(1);
                }
                let tasks = self.visible_tasks().len();
                if self.task_sel >= tasks {
                    self.task_sel = tasks.saturating_sub(1);
                }
            }
            Err(err) => self.status = Some(format!("読み直しに失敗しました: {err}")),
        }
    }

    /// プロジェクトのタスクの状態を 1 段進める。
    fn cycle_project_task(&mut self) {
        let Some(project) = self.selected_project() else {
            self.status = Some("プロジェクトがありません".into());
            return;
        };
        let project_idx = self.visible_projects()[self.project_sel];
        let visible = self.visible_tasks();
        let Some(target) = visible.get(self.task_sel) else {
            self.status = Some("タスクがありません".into());
            return;
        };
        let (title, next) = (target.title.clone(), target.status.next());

        // 並びは現在の表示順のまま、対象だけ状態を変える。
        let mut tasks: Vec<(TaskStatus, String)> = Vec::new();
        for status in [
            TaskStatus::InProgress,
            TaskStatus::Backlog,
            TaskStatus::Done,
        ] {
            for task in project.tasks_with(status) {
                let status = if task.title == title { next } else { status };
                tasks.push((status, task.title.clone()));
            }
        }

        let now = chrono::Local::now().timestamp_millis();
        let project = &self.projects[project_idx];
        if let Err(err) = self.store.save_project_tasks(project, &tasks, now) {
            self.status = Some(format!("保存に失敗しました: {err}"));
            return;
        }
        self.reload_projects();
        // 状態が変わると並びが変わる。カーソルは同じタスクを追う。
        if let Some(at) = self.visible_tasks().iter().position(|t| t.title == title) {
            self.task_sel = at;
        }
        self.status = Some(format!("{title} → {}", next.label()));
    }

    /// 今日の ToDo を編集する。まだ無ければ、進行中のタスクを並べた雛形を出す。
    fn start_today_todo(&mut self) {
        let today = chrono::Local::now().date_naive();
        let date = today.format("%Y-%m-%d").to_string();

        // すでに今日の ToDo があるなら、それをそのまま開く。
        if let Some(note_idx) = self.notes.iter().position(|n| n.date == date) {
            let note = &self.notes[note_idx];
            if let Some(entry_idx) = note.entries.iter().position(|e| note.is_todo(e)) {
                let entry = &note.entries[entry_idx];
                self.pending_edit = Some(EditRequest {
                    target: EditTarget::EntryBody {
                        note_idx,
                        entry_idx,
                    },
                    initial: crate::editor::compose(&entry.tags, &note.body_of(entry).join("\n")),
                });
                self.status = Some("今日の ToDo を開きます".into());
                return;
            }
        }

        // 無ければ、アーカイブしていないプロジェクトの進行中タスクを並べる。
        let groups: Vec<(&str, Vec<&crate::model::ProjectTask>)> = self
            .projects
            .iter()
            .filter(|p| !p.is_archived())
            .map(|p| (p.name.as_str(), p.tasks_with(TaskStatus::InProgress)))
            .filter(|(_, tasks)| !tasks.is_empty())
            .collect();
        let body = crate::model::build_todo_body(today, &groups);
        let count: usize = groups.iter().map(|(_, t)| t.len()).sum();

        self.pending_edit = Some(EditRequest {
            target: EditTarget::NewEntry,
            initial: crate::editor::compose(&[crate::model::TODO_TAG.to_string()], &body),
        });
        self.status = Some(if count > 0 {
            format!("進行中のタスク {count} 件から作ります")
        } else {
            "今日の ToDo を作ります".into()
        });
    }

    fn start_new_entry(&mut self) {
        self.pending_edit = Some(EditRequest {
            target: EditTarget::NewEntry,
            // 1 行目のタグ欄だけ用意して開く。
            initial: crate::editor::compose(&[], ""),
        });
    }

    fn start_delete_entry(&mut self) {
        let Some((note_idx, entry_idx)) = self.focused_entry() else {
            self.status = Some("削除するエントリがありません".into());
            return;
        };
        self.confirm = Some(Confirm::DeleteEntry {
            note_idx,
            entry_idx,
        });
        self.mode = Mode::Confirm;
    }

    fn apply_confirm(&mut self) {
        let Some(confirm) = self.confirm.take() else {
            self.mode = Mode::Normal;
            return;
        };
        self.mode = Mode::Normal;
        match confirm {
            Confirm::DeleteEntry {
                note_idx,
                entry_idx,
            } => {
                let Some(note) = self.notes.get_mut(note_idx) else {
                    return;
                };
                let before = note.to_text();
                if !note.delete_entry(entry_idx) {
                    self.status = Some("エントリが見つかりません".into());
                    return;
                }
                self.detail_scroll = 0;
                self.persist(note_idx, before, "削除しました");
            }
        }
    }

    /// 今日のノートに新しいエントリを足す。ファイルが無ければ作る。
    fn create_entry(&mut self, body: &str, tags: &[String]) {
        let now = chrono::Local::now();
        let date = now.format("%Y-%m-%d").to_string();
        let exists = self.store.note_exists(&date);
        // Acta と同じ規則。その日の最初のエントリは日付だけ、以降は時刻も入れる。
        let created = if exists {
            now.format("%Y-%m-%d %H:%M").to_string()
        } else {
            date.clone()
        };
        let block = format_entry_block(
            &uuid::Uuid::new_v4().to_string(),
            &created,
            now.timestamp_millis(),
            tags,
            body,
        );

        let mut note = match self.store.load_or_create_note(&date) {
            Ok(note) => note,
            Err(err) => {
                self.status = Some(format!("ノートを開けません: {err}"));
                return;
            }
        };
        note.append_entry_block(&block);

        let result = if exists {
            self.store.save_note(&note)
        } else {
            self.store.save_new_note(&note)
        };
        if let Err(err) = result {
            self.status = Some(format!("保存に失敗しました: {err}"));
            return;
        }

        // 一覧に出す位置が変わるので読み直す。
        self.reload();
        self.view = View::Notes;
        self.focus = Focus::Detail;
        self.note_sel = self.notes.iter().position(|n| n.date == date).unwrap_or(0);
        if self.note_sel >= self.visible_notes() {
            self.show_all_notes = true;
        }
        // 追加したエントリは末尾なので、そこまでスクロールする。
        self.detail_scroll = self
            .detail_lines()
            .iter()
            .position(|l| l.entry_idx == self.notes[self.note_sel].entries.len().saturating_sub(1))
            .unwrap_or(0);
        self.status = Some("エントリを追加しました".into());
    }

    /// いま操作対象になっているエントリ。ビューごとに選択の持ち方が違う。
    fn focused_entry(&self) -> Option<(usize, usize)> {
        match self.view {
            View::Notes => {
                let note = self.notes.get(self.note_sel)?;
                if note.entries.is_empty() {
                    return None;
                }
                // 本文ペインのスクロール位置にあるエントリを対象にする。
                let lines = self.detail_lines();
                let entry_idx = lines
                    .get(self.detail_scroll.min(lines.len().saturating_sub(1)))
                    .map(|l| l.entry_idx)
                    .unwrap_or(0);
                Some((self.note_sel, entry_idx))
            }
            View::Todo => {
                let (note_idx, item) = self.todos.get(self.todo_sel)?;
                let note = self.notes.get(*note_idx)?;
                let entry_idx = note
                    .entries
                    .iter()
                    .position(|e| e.body.contains(&item.line))?;
                Some((*note_idx, entry_idx))
            }
            View::Search => {
                let hit = self.hits.get(self.search_sel)?;
                Some((hit.note_idx, hit.entry_idx))
            }
            View::Projects => None,
        }
    }

    fn switch_view(&mut self, view: View) {
        // 検索に入る前のビューを覚えて、Esc で戻れるようにする。
        if view == View::Search && self.view != View::Search {
            self.return_view = self.view;
        }
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

    /// ノート一覧に出す件数。notes は日付降順なので先頭から数えればよい。
    pub fn visible_notes(&self) -> usize {
        if self.show_all_notes || self.config.recent_notes == 0 {
            return self.notes.len();
        }
        self.config.recent_notes.min(self.notes.len())
    }

    /// 一覧に出すプロジェクトの添字。既定ではアーカイブ済みを外す。
    pub fn visible_projects(&self) -> Vec<usize> {
        self.projects
            .iter()
            .enumerate()
            .filter(|(_, p)| self.show_archived || !p.is_archived())
            .map(|(i, _)| i)
            .collect()
    }

    fn apply_move(&mut self, m: Move) {
        // 本文ペインにフォーカスがあるときはスクロール。それ以外は一覧のカーソル。
        if self.view == View::Notes && self.focus == Focus::Detail {
            let max = self.detail_lines().len().saturating_sub(1);
            self.detail_scroll = shift(self.detail_scroll, m, max, self.viewport);
            return;
        }
        // プロジェクトビューで本文側にいるときはタスクを選ぶ。
        if self.view == View::Projects && self.focus == Focus::Detail {
            let len = self.visible_tasks().len();
            if len == 0 {
                self.task_sel = 0;
                return;
            }
            self.task_sel = shift(self.task_sel, m, len - 1, self.viewport);
            return;
        }
        let visible = self.visible_notes();
        let visible_projects = self.visible_projects().len();
        let (sel, len) = match self.view {
            View::Notes => (&mut self.note_sel, visible),
            View::Todo => (&mut self.todo_sel, self.todos.len()),
            View::Projects => (&mut self.project_sel, visible_projects),
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
        if self.view == View::Projects {
            self.task_sel = 0;
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
        let visible = self.visible_notes();
        clamp(&mut self.note_sel, visible);
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
        // 絞り込みの外にあるノートなら、黙って選べないので全件表示にする。
        if hit.note_idx >= self.visible_notes() {
            self.show_all_notes = true;
        }
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
                format!("  {}", entry.tags.join(" · "))
            };
            // エントリの頭に区切りを置くと、どこから始まるか目で追える。
            out.push(DetailLine {
                text: format!("── {}{}", short_time(&entry.created), tags),
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
        let visible = self.visible_projects();
        self.projects.get(*visible.get(self.project_sel)?)
    }

    /// 右ペインに出すタスク。Done は数を絞る。並びは画面の順と揃える。
    pub fn visible_tasks(&self) -> Vec<&crate::model::ProjectTask> {
        let Some(project) = self.selected_project() else {
            return Vec::new();
        };
        let mut out = Vec::new();
        for status in [
            TaskStatus::InProgress,
            TaskStatus::Backlog,
            TaskStatus::Done,
        ] {
            let tasks = project.tasks_with(status);
            let shown = if status == TaskStatus::Done && self.config.project_done_limit > 0 {
                self.config.project_done_limit.min(tasks.len())
            } else {
                tasks.len()
            };
            out.extend(tasks.into_iter().take(shown));
        }
        out
    }

    /// 表示から省いた Done の件数。
    pub fn hidden_done(&self) -> usize {
        let Some(project) = self.selected_project() else {
            return 0;
        };
        let limit = self.config.project_done_limit;
        if limit == 0 {
            return 0;
        }
        project.count(TaskStatus::Done).saturating_sub(limit)
    }

    /// 画面下部に出す集計。
    pub fn summary(&self) -> String {
        let open = self
            .todos
            .iter()
            .filter(|(_, t)| t.status != TaskStatus::Done)
            .count();
        let active = self.projects.iter().filter(|p| !p.is_archived()).count();
        let visible = self.visible_notes();
        let notes = if visible < self.notes.len() {
            format!("ノート {}/{}", visible, self.notes.len())
        } else {
            format!("ノート {}", self.notes.len())
        };
        format!("{notes} / ToDo 未完 {open} / プロジェクト {active}")
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
