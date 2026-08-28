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

/// ToDo 一覧の並び順。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TodoSort {
    /// 新しい日付から。既定。
    Date,
    /// 未着手・進行中・完了の順。
    Status,
    /// プロジェクト名の順。
    Project,
}

impl TodoSort {
    pub fn label(self) -> &'static str {
        match self {
            Self::Date => "日付順",
            Self::Status => "状態順",
            Self::Project => "プロジェクト順",
        }
    }

    pub fn next(self) -> Self {
        match self {
            Self::Date => Self::Status,
            Self::Status => Self::Project,
            Self::Project => Self::Date,
        }
    }
}

/// ビュー内のフォーカス。左から順に並べてある。
///
/// ノートビューだけ 3 ペインで、日付 → エントリ → 本文と移る。
/// 他のビューは `List` と `Detail` の 2 つだけを使う。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Focus {
    List,
    /// ノートビューの中央ペイン（エントリ一覧）。
    Entries,
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
    /// フォーカスを右のペインへ。端では止まる。
    FocusRight,
    /// フォーカスを左のペインへ。端では止まる。
    FocusLeft,
    /// フォーカスを右へ 1 つ。一番右まで行ったら左端へ戻る。
    FocusCycle,
    /// ToDo のチェック状態を 1 段進める。
    CycleTodo,
    Reload,
    /// ノート一覧を直近だけにするか、全件にするか。
    ToggleAllNotes,
    /// アーカイブ済みのプロジェクトを出すかどうか。
    ToggleArchived,
    /// ToDo 一覧の並び順を変える。
    CycleTodoSort,
    /// ToDo 一覧を未完だけにするかどうか。
    ToggleTodoFilter,
    /// 選択中のエントリを $EDITOR で編集する。
    EditEntry,
    /// 今日のノートに新しいエントリを作る。
    NewEntry,
    /// 今日の ToDo を開く。無ければ進行中のタスクから作る。
    TodayTodo,
    /// プロジェクトを新しく作る。
    NewProject,
    /// 選択中のプロジェクトを 1 つ下 / 上へ動かす。
    MoveProject(i32),
    /// 選択中のプロジェクトをアーカイブする / 戻す。
    ToggleArchiveProject,
    /// 表示中の ToDo を Markdown でクリップボードへ。
    CopyTodo,
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

/// ToDo ビューの右ペインに並べる行。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TodoRow {
    /// プロジェクト名の見出し。選択の対象にはしない。
    Group(String),
    /// `visible_todos()` の何番目か。
    Task(usize),
}

/// 本文ペインの 1 行。描画とスクロール位置の計算で共用する。
///
/// 本文ペインに出るのは選択中のエントリ 1 件だけなので、どのエントリの行かは持たない。
#[derive(Debug, Clone)]
pub struct DetailLine {
    pub text: String,
    pub kind: LineKind,
    /// ToDo のタスク行なら、ファイル内の行番号。本文で選んで進めるのに使う。
    pub task_line: Option<usize>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LineKind {
    Heading,
    ListItem,
    Quote,
    Code,
    Body,
}

/// ToDo の 1 行を (プロジェクト名, タイトル, 状態) で表したもの。
/// プロジェクトへ書き戻すときの受け渡しに使う。
type TodoRowRef = (String, String, TaskStatus);

/// プロジェクトへ反映した結果。状態行に出す文言を組み立てる。
#[derive(Debug, Clone, Copy, Default)]
struct ProjectSync {
    /// 書き換えたプロジェクトの数。
    projects: usize,
    added: usize,
    removed: usize,
}

impl ProjectSync {
    fn detail(self) -> Option<String> {
        if self.projects == 0 {
            return None;
        }
        let mut out = format!("プロジェクト {} 件に反映", self.projects);
        if self.added > 0 {
            out.push_str(&format!("・{} 件追加", self.added));
        }
        if self.removed > 0 {
            out.push_str(&format!("・{} 件削除", self.removed));
        }
        Some(out)
    }
}

/// 中央ペインに出すエントリ 1 件の見出し。
#[derive(Debug, Clone)]
pub struct EntrySummary {
    /// 時刻。Acta が書いたその日の最初のエントリは日付だけなので、無いこともある。
    pub time: Option<String>,
    pub title: String,
    pub tags: Vec<String>,
    pub todo: bool,
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
    /// ノートビューの中央ペイン（エントリ一覧）のカーソル。
    pub entry_sel: usize,
    pub detail_scroll: usize,
    /// 本文ペインで選んでいる ToDo タスクの通し番号。
    pub note_task_sel: usize,
    /// ToDo ビューの左ペイン（日付）のカーソル。
    pub todo_date_sel: usize,
    /// ToDo ビューの右ペイン（タスク）のカーソル。
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
    /// ToDo 一覧の並び順。
    pub todo_sort: TodoSort,
    /// ToDo 一覧を未完だけにしているか。
    pub todo_open_only: bool,
    /// ヘルプ画面のスクロール位置。低い端末でも全部読めるようにする。
    pub help_scroll: usize,
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
            entry_sel: 0,
            detail_scroll: 0,
            note_task_sel: 0,
            todo_date_sel: 0,
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
            todo_sort: TodoSort::Date,
            todo_open_only: false,
            help_scroll: 0,
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
                self.help_scroll = 0;
            }
            Msg::SwitchView(view) => self.switch_view(view),
            Msg::NextView => self.cycle_view(1),
            Msg::PrevView => self.cycle_view(-1),
            Msg::FocusRight => self.move_focus(1),
            Msg::FocusLeft => self.move_focus(-1),
            Msg::FocusCycle => self.cycle_focus(),
            Msg::Move(m) => self.apply_move(m),
            Msg::CycleTodo => {
                if self.view == View::Projects {
                    self.cycle_project_task();
                } else {
                    self.cycle_todo();
                }
            }
            Msg::Reload => self.reload(),
            Msg::CycleTodoSort => {
                self.todo_sort = self.todo_sort.next();
                self.keep_todo_selection();
                self.status = Some(format!("ToDo を{}に並べます", self.todo_sort.label()));
            }
            Msg::ToggleTodoFilter => {
                self.todo_open_only = !self.todo_open_only;
                self.keep_todo_selection();
                self.status = Some(if self.todo_open_only {
                    format!("未完の {} 件だけ表示", self.visible_todos().len())
                } else {
                    format!("すべての {} 件を表示", self.visible_todos().len())
                });
            }
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
            Msg::NewEntry => {
                if self.view == View::Projects {
                    self.update(Msg::NewProject);
                } else {
                    self.start_new_entry();
                }
            }
            Msg::TodayTodo => self.start_today_todo(),
            Msg::MoveProject(delta) => self.move_project(delta),
            Msg::ToggleArchiveProject => self.toggle_archive_project(),
            Msg::CopyTodo => self.copy_todo(),
            Msg::NewProject => {
                self.pending_edit = Some(EditRequest {
                    target: EditTarget::NewProject,
                    initial: String::new(),
                });
                self.status = Some("プロジェクト名を 1 行で書いて保存します".into());
            }
            Msg::DeleteEntry => self.start_delete_entry(),
            Msg::ConfirmYes => self.apply_confirm(),
            Msg::ConfirmNo => {
                self.confirm = None;
                self.mode = Mode::Normal;
            }
        }
    }

    /// そのビューのペインを左から並べたもの。ノートビューだけ 3 ペインある。
    fn focus_lanes(&self) -> &'static [Focus] {
        if self.view == View::Notes {
            &[Focus::List, Focus::Entries, Focus::Detail]
        } else {
            &[Focus::List, Focus::Detail]
        }
    }

    /// フォーカスを隣のペインへ移す。端では止まる。h / l 用。
    fn move_focus(&mut self, delta: i32) {
        let lanes = self.focus_lanes();
        let at = lanes.iter().position(|f| *f == self.focus).unwrap_or(0) as i32;
        let next = (at + delta).clamp(0, lanes.len() as i32 - 1) as usize;
        self.focus = lanes[next];
    }

    /// フォーカスを右へ 1 つ。一番右まで行ったら左端へ戻る。
    ///
    /// Enter だけで一周できるようにする。右端で押しても何も起きないと、
    /// 戻り方が分からないまま行き止まりになる。
    fn cycle_focus(&mut self) {
        let lanes = self.focus_lanes();
        let at = lanes.iter().position(|f| *f == self.focus).unwrap_or(0);
        self.focus = lanes[(at + 1) % lanes.len()];
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
            EditTarget::NewProject => self.create_project(&text),
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
        // 編集前の ToDo 行を控えておく。あとで見比べて増減をプロジェクトへ送る。
        let before_rows = note.entry_todo_rows(entry_idx);
        let before = note.to_text();
        if !note.replace_entry_body(entry_idx, &body) {
            self.status = Some("エントリが見つかりません".into());
            return;
        }
        // タグも 1 行目で編集できるので、変わっていれば反映する。
        if note.entries[entry_idx].tags != tags {
            note.set_entry_tags(entry_idx, &tags);
        }
        // ToDo でなくなったときは何もしない。タグを消しただけで
        // プロジェクトのタスクが消えてしまうと取り返しがつかない。
        let still_todo = note.entries.get(entry_idx).is_some_and(|e| note.is_todo(e));
        let after_rows = note.entry_todo_rows(entry_idx);
        self.persist(note_idx, before, "保存しました");
        if !still_todo {
            return;
        }
        // エディタで書き換えた分をプロジェクトへ送る。増減もそのまま反映する。
        let sync = self.apply_project_changes(&before_rows, &after_rows, true);
        if let Some(detail) = sync.detail() {
            self.status = Some(format!("保存しました（{detail}）"));
        }
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
            let checklist = self.projects[project_idx].tasks_as_checklist();
            self.pending_edit = Some(EditRequest {
                target: EditTarget::ProjectTasks { project_idx },
                // タスクが 1 件も無いと何を書けばよいか分からないので、行の形を出す。
                initial: if checklist.is_empty() {
                    "- [ ] ".to_string()
                } else {
                    checklist
                },
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

    /// 一覧の並びを 1 つ入れ替える。
    ///
    /// 並び順はデータディレクトリの .nota に持つ。project.json に足すと、
    /// Acta がそのプロジェクトを保存したときに落ちてしまう。
    fn move_project(&mut self, delta: i32) {
        let visible = self.visible_projects();
        if visible.len() < 2 {
            return;
        }
        let at = self.project_sel;
        let to = at as i32 + delta;
        if to < 0 || to as usize >= visible.len() {
            return;
        }
        let to = to as usize;

        // 表示中の並びを基準に入れ替える。隠しているものは今の位置に残す。
        let mut order: Vec<String> = self.projects.iter().map(|p| p.dir_name.clone()).collect();
        let from_idx = visible[at];
        let to_idx = visible[to];
        let (from_pos, to_pos) = (
            order
                .iter()
                .position(|d| *d == self.projects[from_idx].dir_name),
            order
                .iter()
                .position(|d| *d == self.projects[to_idx].dir_name),
        );
        let (Some(from_pos), Some(to_pos)) = (from_pos, to_pos) else {
            return;
        };
        let moved = order.remove(from_pos);
        order.insert(to_pos, moved);

        if let Err(err) = self.store.save_project_order(&order) {
            self.status = Some(format!("並びを保存できません: {err}"));
            return;
        }
        self.reload_projects();
        // reload_projects は選択を名前で追うが、ここでは動かした先に置きたい。
        self.project_sel = to;
        self.status = Some(format!(
            "{} を {} へ",
            self.selected_project()
                .map(|p| p.name.as_str())
                .unwrap_or(""),
            if delta > 0 { "下" } else { "上" }
        ));
    }

    /// 選択中のプロジェクトをアーカイブする / 戻す。
    fn toggle_archive_project(&mut self) {
        if self.view != View::Projects {
            self.status = Some("プロジェクトビューで切り替えます".into());
            return;
        }
        let Some(idx) = self.visible_projects().get(self.project_sel).copied() else {
            self.status = Some("プロジェクトがありません".into());
            return;
        };
        let project = &self.projects[idx];
        let (name, archived) = (project.name.clone(), project.is_archived());
        let now = chrono::Local::now().timestamp_millis();
        // 戻すときは 0 を書く。Acta もアーカイブの有無をこの値だけで見ている。
        let at = if archived { 0 } else { now };
        if let Err(err) = self.store.set_project_archived(project, at, now) {
            self.status = Some(format!("保存に失敗しました: {err}"));
            return;
        }
        self.reload_projects();
        // アーカイブすると一覧から消えることがある。選択を収める。
        let len = self.visible_projects().len();
        if self.project_sel >= len {
            self.project_sel = len.saturating_sub(1);
        }
        self.task_sel = 0;
        self.status = Some(if archived {
            format!("{name} をアーカイブから戻しました")
        } else {
            format!("{name} をアーカイブしました（A で表示）")
        });
    }

    /// 表示中の ToDo を Markdown にしてクリップボードへ。
    fn copy_todo(&mut self) {
        if self.view != View::Todo {
            self.status = Some("ToDo ビュー（2）でコピーできます".into());
            return;
        }
        let Some(text) = self.todo_markdown() else {
            self.status = Some("コピーする ToDo がありません".into());
            return;
        };
        let rows = text.lines().count();
        match crate::clipboard::copy(&text) {
            Ok(command) => {
                self.status = Some(format!(
                    "ToDo を Markdown でコピーしました（{rows} 行 / {command}）"
                ))
            }
            Err(err) => self.status = Some(format!("コピーできません: {err}")),
        }
    }

    /// いま右ペインに出ている ToDo を Markdown にする。
    ///
    /// 並び替えや絞り込みをそのまま反映する。画面で整えたものが、
    /// そのまま貼り付けられるようにするため。
    pub fn todo_markdown(&self) -> Option<String> {
        let date = self.selected_todo_date()?;
        let todos = self.visible_todos();
        if todos.is_empty() {
            return None;
        }
        let heading = match chrono::NaiveDate::parse_from_str(&date, "%Y-%m-%d") {
            Ok(day) => crate::model::todo_heading(day),
            Err(_) => format!("ToDo: {date}"),
        };
        let mut out = vec![format!("# {heading}")];
        for row in self.todo_rows() {
            match row {
                TodoRow::Group(name) => out.push(format!("- {name}")),
                TodoRow::Task(at) => {
                    let item = &todos[at].1;
                    // プロジェクトの無い行は見出しが出ないので、字下げもしない。
                    let indent = if item.group.is_empty() { "" } else { "  " };
                    out.push(format!(
                        "{indent}- [{}] {}",
                        item.status.marker(),
                        item.title
                    ));
                }
            }
        }
        Some(out.join("\n"))
    }

    /// 名前を受け取ってプロジェクトを作る。最初の中身のある行だけを見る。
    fn create_project(&mut self, text: &str) {
        let Some(name) = text.lines().map(str::trim).find(|l| !l.is_empty()) else {
            self.status = Some("名前が空なので作成しませんでした".into());
            return;
        };
        let now = chrono::Local::now().timestamp_millis();
        match self.store.create_project(name, now) {
            Ok(dir_name) => {
                self.reload_projects();
                // 作ったものを選んだ状態にする。すぐタスクを足せる。
                self.view = View::Projects;
                self.focus = Focus::List;
                if let Some(at) = self
                    .visible_projects()
                    .iter()
                    .position(|i| self.projects[*i].dir_name == dir_name)
                {
                    self.project_sel = at;
                    self.task_sel = 0;
                }
                self.status = Some(format!("{name} を作成しました（e でタスクを足せます）"));
            }
            Err(err) => self.status = Some(format!("作成に失敗しました: {err}")),
        }
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
        if tasks.is_empty() {
            self.status = Some("タスクとして読み取れる行がありませんでした".into());
            return;
        }
        // 今日の ToDo から消す行を控えておく。
        let before: Vec<String> = project.tasks.iter().map(|t| t.title.clone()).collect();
        let removed: Vec<String> = before
            .iter()
            .filter(|title| !tasks.iter().any(|(_, t)| t == *title))
            .cloned()
            .collect();

        let now = chrono::Local::now().timestamp_millis();
        if let Err(err) = self.store.save_project_tasks(project, &tasks, now) {
            self.status = Some(format!("保存に失敗しました: {err}"));
            return;
        }
        let name = project.name.clone();
        self.reload_projects();
        self.status = Some(format!("タスクを保存しました（{} 件）", tasks.len()));
        self.sync_today_todo(&name, &removed);
    }

    /// プロジェクトのタスクを今日の ToDo に流し込む。
    ///
    /// 今日の ToDo に出すのは進行中のタスクだけ。未着手まで並べると、
    /// まだ手を付けないつもりの予定でその日が埋まってしまう。
    ///
    /// `removed` はタスク一覧を編集して消えた行。その行は ToDo からも消す。
    ///
    /// 今日の ToDo がまだ無いときは作らない。プロジェクトをいじるたびに
    /// ノートが勝手に増えるのは驚きが大きいので、作るのは t に任せる。
    fn sync_today_todo(&mut self, project_name: &str, removed: &[String]) {
        // 添字で持ち回らない。保存で updatedAtMs が変わると一覧の並びが変わる。
        let Some(project) = self.projects.iter().find(|p| p.name == project_name) else {
            return;
        };
        let name = project.name.clone();

        // 進行中は足す。それ以外は、すでに ToDo にある行の状態を合わせるだけ。
        let tasks: Vec<(TaskStatus, String, bool)> = project
            .tasks
            .iter()
            .map(|t| {
                (
                    t.status,
                    t.title.clone(),
                    t.status == TaskStatus::InProgress,
                )
            })
            .collect();
        if tasks.is_empty() && removed.is_empty() {
            return;
        }

        let date = chrono::Local::now().format("%Y-%m-%d").to_string();
        let today_todo = self
            .notes
            .iter()
            .position(|n| n.date == date)
            .and_then(|at| {
                let note = &self.notes[at];
                note.entries
                    .iter()
                    .position(|e| note.is_todo(e))
                    .map(|entry| (at, entry))
            });
        let Some((note_idx, entry_idx)) = today_todo else {
            // 黙って落とさない。t で作れば入ることが分かるようにする。
            if tasks.iter().any(|(_, _, append)| *append) {
                self.status = Some(
                    "タスクを保存しました（今日の ToDo がまだ無いので反映していません。t で作れます）"
                        .to_string(),
                );
            }
            return;
        };

        let note = &self.notes[note_idx];
        let body: Vec<String> = note
            .body_of(&note.entries[entry_idx])
            .into_iter()
            .map(str::to_string)
            .collect();
        let (next, put, updated, dropped) =
            crate::model::upsert_todo_group(&body, &name, &tasks, removed);
        if put == 0 && updated == 0 && dropped == 0 {
            return;
        }

        let note = &mut self.notes[note_idx];
        let before = note.to_text();
        if !note.replace_entry_body(entry_idx, &next.join("\n")) {
            return;
        }
        // 何をしたかを 1 行にまとめる。0 件のものは出さない。
        let parts: Vec<String> = [(put, "追加"), (updated, "更新"), (dropped, "削除")]
            .into_iter()
            .filter(|(count, _)| *count > 0)
            .map(|(count, label)| format!("{count} 件{label}"))
            .collect();
        let detail = format!("今日の ToDo に {}しました", parts.join("、"));
        self.persist(note_idx, before, &detail);
    }

    /// プロジェクトだけ読み直す。ノートまで読み直す必要はない。
    fn reload_projects(&mut self) {
        // 保存で updatedAtMs が変わると並びが変わる。選択は名前で追う。
        let selected = self.selected_project().map(|p| p.name.clone());
        match self.store.load_projects() {
            Ok(projects) => {
                self.projects = projects;
                let visible = self.visible_projects();
                if let Some(name) = selected {
                    if let Some(at) = visible.iter().position(|i| self.projects[*i].name == name) {
                        self.project_sel = at;
                    }
                }
                let len = visible.len();
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
        // 左ペインにいるときは、どのタスクが対象か画面から読み取れない。
        // 一番上を黙って変えてしまわないよう、まずタスク側へ移る。
        if self.focus == Focus::List {
            if self.visible_tasks().is_empty() {
                self.status = Some("タスクがありません".into());
                return;
            }
            self.focus = Focus::Detail;
            self.status = Some("タスクを選んで Space で状態を進めます".into());
            return;
        }
        let Some(project) = self.selected_project() else {
            self.status = Some("プロジェクトがありません".into());
            return;
        };
        let project_name = project.name.clone();
        let project_idx = self.visible_projects()[self.project_sel];
        let visible = self.visible_tasks();
        let Some(target) = visible.get(self.task_sel) else {
            self.status = Some("タスクがありません".into());
            return;
        };
        let (title, next) = (target.title.clone(), target.status.next());

        // 並びは現在の表示順のまま、対象だけ状態を変える。
        let mut tasks: Vec<(TaskStatus, String)> = Vec::new();
        for status in crate::model::STATUS_ORDER {
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
        // Space は状態を変えるだけ。行の増減は起きない。
        self.sync_today_todo(&project_name, &[]);
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
                self.note_task_sel = 0;
                let len = self.notes[note_idx].entries.len();
                if self.entry_sel >= len {
                    self.entry_sel = len.saturating_sub(1);
                }
                self.persist(note_idx, before, "削除しました");
            }
        }
    }

    /// 今日のノートに新しいエントリを足す。ファイルが無ければ作る。
    fn create_entry(&mut self, body: &str, tags: &[String]) {
        let now = chrono::Local::now();
        let date = now.format("%Y-%m-%d").to_string();
        let exists = self.store.note_exists(&date);
        // その日の最初のエントリでも時刻を入れる。Acta は日付だけを書くが、
        // それだと ToDo を作った時刻が分からなくなる。読む側は同じ形で解釈できる。
        let created = now.format("%Y-%m-%d %H:%M").to_string();
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
        // 追加したエントリは末尾なので、そこを選んだ状態にする。
        self.entry_sel = self.notes[self.note_sel].entries.len().saturating_sub(1);
        self.detail_scroll = 0;
        self.note_task_sel = 0;
        self.status = Some("エントリを追加しました".into());
    }

    /// いま操作対象になっているエントリ。ビューごとに選択の持ち方が違う。
    fn focused_entry(&self) -> Option<(usize, usize)> {
        match self.view {
            View::Notes => {
                let note = self.notes.get(self.note_sel)?;
                // 中央ペインで選んでいるエントリがそのまま対象になる。
                (self.entry_sel < note.entries.len()).then_some((self.note_sel, self.entry_sel))
            }
            View::Todo => {
                let (note_idx, item) = self.visible_todos().get(self.todo_sel).copied()?;
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
        if self.mode == Mode::Help {
            // 行数は描画側が知っているので、ここでは上限を決めず送るだけにする。
            self.help_scroll = shift(self.help_scroll, m, usize::MAX - 1, 5);
            return;
        }
        // 中央ペインにいるときは、その日のエントリを選ぶ。
        if self.view == View::Notes && self.focus == Focus::Entries {
            let len = self.selected_note().map(|n| n.entries.len()).unwrap_or(0);
            if len == 0 {
                self.entry_sel = 0;
                return;
            }
            self.entry_sel = shift(self.entry_sel, m, len - 1, self.viewport);
            // エントリを移ったら本文側の位置は先頭に戻す。
            self.detail_scroll = 0;
            self.note_task_sel = 0;
            return;
        }
        // 本文ペインにフォーカスがあるとき。ToDo のタスクがあればその行を選び、
        // 無ければ今までどおりスクロールする。
        if self.view == View::Notes && self.focus == Focus::Detail {
            let tasks = self.note_task_positions();
            if tasks.is_empty() {
                let max = self.detail_lines().len().saturating_sub(1);
                self.detail_scroll = shift(self.detail_scroll, m, max, self.viewport);
                return;
            }
            self.note_task_sel = shift(self.note_task_sel, m, tasks.len() - 1, self.viewport);
            self.scroll_to_note_task();
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
        // ToDo ビューも同じ形。右にいるときはその日のタスクを選ぶ。
        if self.view == View::Todo && self.focus == Focus::Detail {
            let len = self.visible_todos().len();
            if len == 0 {
                self.todo_sel = 0;
                return;
            }
            self.todo_sel = shift(self.todo_sel, m, len - 1, self.viewport);
            return;
        }
        let visible = self.visible_notes();
        let visible_projects = self.visible_projects().len();
        let visible_dates = self.visible_todo_dates().len();
        let (sel, len) = match self.view {
            View::Notes => (&mut self.note_sel, visible),
            View::Todo => (&mut self.todo_date_sel, visible_dates),
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
            // 日を移ったらエントリのカーソルも先頭に戻す。
            self.entry_sel = 0;
            self.detail_scroll = 0;
            self.note_task_sel = 0;
        }
        if self.view == View::Projects {
            self.task_sel = 0;
        }
        // 日を移ったらタスクのカーソルは先頭に戻す。
        if self.view == View::Todo {
            self.todo_sel = 0;
        }
    }

    fn cycle_todo(&mut self) {
        // ノートビューでは、本文に出ているタスク行をそのまま進める。
        if self.view == View::Notes {
            self.cycle_note_task();
            return;
        }
        // 日付側にいるときは、どのタスクが対象か画面から読み取れない。
        // プロジェクトビューと同じく、まずタスク側へ移る。
        if self.view == View::Todo && self.focus == Focus::List {
            if self.visible_todos().is_empty() {
                self.status = Some("ToDo がありません".into());
                return;
            }
            self.focus = Focus::Detail;
            self.status = Some("タスクを選んで Space で状態を進めます".into());
            return;
        }
        let Some((note_idx, item)) = self
            .visible_todos()
            .get(self.todo_sel)
            .map(|x| (*x).clone())
        else {
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
        // ToDo の行がプロジェクトのタスクなら、そちらの状態も合わせる。
        if self.sync_projects_from_rows(&[(item.group.clone(), item.title.clone(), next)]) > 0 {
            self.status = Some(format!(
                "{} → {}（{} にも反映）",
                item.title,
                next.label(),
                item.group
            ));
        }
    }

    /// ToDo の行の状態をプロジェクトのタスクに書き戻す。
    ///
    /// 行のグループ名とタスク名で探す。手で足した行のようにプロジェクトに
    /// 対応が無いものは、そのまま ToDo だけの予定として置いておく。
    /// 戻り値は書き換えたプロジェクトの数。
    fn sync_projects_from_rows(&mut self, rows: &[TodoRowRef]) -> usize {
        self.apply_project_changes(rows, rows, false).projects
    }

    /// ToDo の行をプロジェクトのタスクへ反映する。
    ///
    /// `before` と `after` を見比べるので、状態だけでなく行の増減も送れる。
    /// `allow_add_remove` が false のときは状態だけを合わせる。Space のように
    /// 1 行だけ触った操作で、他の行が消えてしまわないようにするため。
    fn apply_project_changes(
        &mut self,
        before: &[TodoRowRef],
        after: &[TodoRowRef],
        allow_add_remove: bool,
    ) -> ProjectSync {
        // プロジェクトごとにまとめてから書く。1 行ずつ保存すると、
        // 同じファイルを何度も開くうえに更新時刻も無駄に動く。
        let mut groups: Vec<String> = Vec::new();
        for (group, _, _) in after.iter().chain(before.iter()) {
            if !group.is_empty() && !groups.contains(group) {
                groups.push(group.clone());
            }
        }

        let mut sync = ProjectSync::default();
        for group in groups {
            let Some(idx) = self.projects.iter().position(|p| p.name == group) else {
                continue;
            };
            // ToDo から消えた行。編集前にあって、いま無いものだけを対象にする。
            // 元から ToDo に出ていなかったタスクは触らない。
            let removed: Vec<&str> = if allow_add_remove {
                before
                    .iter()
                    .filter(|(g, title, _)| {
                        *g == group && !after.iter().any(|(g2, t2, _)| *g2 == group && t2 == title)
                    })
                    .map(|(_, title, _)| title.as_str())
                    .collect()
            } else {
                Vec::new()
            };

            let project = &self.projects[idx];
            // 残すタスクは並びも項目もそのまま。状態だけ ToDo 側に合わせる。
            let mut tasks: Vec<(TaskStatus, String)> = project
                .tasks
                .iter()
                .filter(|t| !removed.contains(&t.title.as_str()))
                .map(|t| {
                    let next = after
                        .iter()
                        .find(|(g, title, _)| *g == group && *title == t.title)
                        .map(|(_, _, status)| *status)
                        .unwrap_or(t.status);
                    (next, t.title.clone())
                })
                .collect();
            // ToDo で足した行はプロジェクトの末尾に置く。
            let mut added = 0;
            if allow_add_remove {
                for (g, title, status) in after {
                    if *g != group || tasks.iter().any(|(_, t)| t == title) {
                        continue;
                    }
                    tasks.push((*status, title.clone()));
                    added += 1;
                }
            }

            // すでに同じ内容なら書かない。更新時刻をむやみに動かさない。
            let same = added == 0
                && removed.is_empty()
                && tasks
                    .iter()
                    .zip(project.tasks.iter())
                    .all(|((next, _), t)| *next == t.status);
            if same {
                continue;
            }

            let now = chrono::Local::now().timestamp_millis();
            if let Err(err) = self
                .store
                .save_project_tasks(&self.projects[idx], &tasks, now)
            {
                self.status = Some(format!("プロジェクトに反映できません: {err}"));
                continue;
            }
            sync.projects += 1;
            sync.added += added;
            sync.removed += removed.len();
        }

        if sync.projects > 0 {
            self.reload_projects();
        }
        sync
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
        let entries = self.selected_note().map(|n| n.entries.len()).unwrap_or(0);
        clamp(&mut self.entry_sel, entries);
        let visible = self.visible_todos().len();
        clamp(&mut self.todo_sel, visible);
        clamp(&mut self.project_sel, self.projects.len());
        clamp(&mut self.search_sel, self.hits.len());
        self.detail_scroll = 0;
    }

    /// 絞り込みを通ったあとの ToDo。日付一覧もタスク一覧もここから作る。
    fn filtered_todos(&self) -> Vec<&(usize, TodoItem)> {
        self.todos
            .iter()
            .filter(|(_, item)| !self.todo_open_only || item.status != TaskStatus::Done)
            .collect()
    }

    /// 左ペインに出す日付。新しい順で、既定は直近だけ。
    ///
    /// 古い ToDo まで並べても探しにくいので、ノート一覧と同じ考え方で絞る。
    /// `a` を押すと全部出る。
    pub fn visible_todo_dates(&self) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        for (_, item) in self.filtered_todos() {
            if !out.contains(&item.date) {
                out.push(item.date.clone());
            }
        }
        if !self.show_all_notes && self.config.recent_notes > 0 {
            out.truncate(self.config.recent_notes);
        }
        out
    }

    /// 絞らなければ何日ぶんあるか。見出しに「30/58」と出すために使う。
    pub fn todo_dates_total(&self) -> usize {
        let mut out: Vec<&str> = Vec::new();
        for (_, item) in self.filtered_todos() {
            if !out.contains(&item.date.as_str()) {
                out.push(&item.date);
            }
        }
        out.len()
    }

    /// いま選んでいる日付。
    pub fn selected_todo_date(&self) -> Option<String> {
        self.visible_todo_dates().get(self.todo_date_sel).cloned()
    }

    /// 右ペインに出す ToDo。選んだ日のぶんだけを、指定の並びで返す。
    pub fn visible_todos(&self) -> Vec<&(usize, TodoItem)> {
        let Some(date) = self.selected_todo_date() else {
            return Vec::new();
        };
        let mut out: Vec<&(usize, TodoItem)> = self
            .filtered_todos()
            .into_iter()
            .filter(|(_, item)| item.date == date)
            .collect();
        // ファイルに書かれた順がそのまま入っている。並べ替えは安定ソートなので、
        // 同じ値の中では元の順が残る。
        match self.todo_sort {
            TodoSort::Date => {}
            TodoSort::Status => out.sort_by_key(|(_, item)| match item.status {
                TaskStatus::Backlog => 0,
                TaskStatus::InProgress => 1,
                TaskStatus::Done => 2,
            }),
            TodoSort::Project => out.sort_by(|(_, a), (_, b)| a.group.cmp(&b.group)),
        }
        out
    }

    /// 右ペインの行。プロジェクト名の見出しを挟んで階層に見せる。
    pub fn todo_rows(&self) -> Vec<TodoRow> {
        let mut out = Vec::new();
        let mut current: Option<String> = None;
        for (at, (_, item)) in self.visible_todos().iter().enumerate() {
            // 見出しは並びの中でグループが切り替わったところに出す。
            if current.as_deref() != Some(item.group.as_str()) {
                if !item.group.is_empty() {
                    out.push(TodoRow::Group(item.group.clone()));
                }
                current = Some(item.group.clone());
            }
            out.push(TodoRow::Task(at));
        }
        out
    }

    /// 並びや絞り込みを変えても、選んでいた行を見失わないようにする。
    fn keep_todo_selection(&mut self) {
        let current = self
            .visible_todos()
            .get(self.todo_sel)
            .map(|(_, item)| (item.date.clone(), item.line));
        // 日付の並びも変わりうるので、まず日付側を収める。
        let dates = self.visible_todo_dates();
        if self.todo_date_sel >= dates.len() {
            self.todo_date_sel = dates.len().saturating_sub(1);
        }
        let next = self.visible_todos();
        self.todo_sel = current
            .and_then(|(date, line)| {
                next.iter()
                    .position(|(_, i)| i.date == date && i.line == line)
            })
            .unwrap_or(0)
            .min(next.len().saturating_sub(1));
    }

    pub fn rebuild_todos(&mut self) {
        let mut out = Vec::new();
        for (idx, note) in self.notes.iter().enumerate() {
            for item in note.todo_items() {
                out.push((idx, item));
            }
        }
        self.todos = out;
        let visible = self.visible_todos().len();
        clamp(&mut self.todo_sel, visible);
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
        self.entry_sel = hit.entry_idx;
        self.detail_scroll = 0;
        self.note_task_sel = 0;
    }

    pub fn selected_note(&self) -> Option<&DailyNote> {
        self.notes.get(self.note_sel)
    }

    /// 本文ペインの行。選択中のエントリ 1 件ぶんだけを返す。
    pub fn detail_lines(&self) -> Vec<DetailLine> {
        self.entry_lines(self.note_sel, self.entry_sel)
    }

    fn entry_lines(&self, note_idx: usize, entry_idx: usize) -> Vec<DetailLine> {
        let Some(note) = self.notes.get(note_idx) else {
            return Vec::new();
        };
        let Some(entry) = note.entries.get(entry_idx) else {
            return Vec::new();
        };
        // ToDo のタスク行はファイル行番号で覚えておき、本文からも進められるようにする。
        let task_lines: Vec<usize> = note.todo_items().iter().map(|i| i.line).collect();

        let mut out = Vec::new();
        let mut in_code = false;
        for (offset, line) in note.body_of(entry).into_iter().enumerate() {
            let file_line = entry.body.start + offset;
            let trimmed = line.trim_start();
            if trimmed.starts_with("```") {
                in_code = !in_code;
                out.push(DetailLine {
                    text: line.to_string(),
                    kind: LineKind::Code,
                    task_line: None,
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
                // コードブロックの中の似た行は対象外。todo_items が拾った行だけ。
                task_line: (!in_code && task_lines.contains(&file_line)).then_some(file_line),
            });
        }
        out
    }

    /// 中央ペインに出すエントリの見出し。時刻・タグ・本文の 1 行目。
    pub fn entry_summaries(&self) -> Vec<EntrySummary> {
        let Some(note) = self.selected_note() else {
            return Vec::new();
        };
        note.entries
            .iter()
            .map(|entry| {
                let title = note
                    .body_of(entry)
                    .into_iter()
                    .map(|l| l.trim().trim_start_matches('#').trim())
                    .find(|l| !l.is_empty())
                    .unwrap_or("(空)")
                    .chars()
                    .take(80)
                    .collect();
                EntrySummary {
                    time: short_time(&entry.created),
                    title,
                    tags: entry.tags.clone(),
                    todo: note.is_todo(entry),
                }
            })
            .collect()
    }

    /// ノートの本文で選んでいるタスクの状態を進める。
    ///
    /// ToDo ビューと同じく、書き戻したあとプロジェクトへも送る。
    fn cycle_note_task(&mut self) {
        if self.focus == Focus::List {
            if self.note_task_positions().is_empty() {
                self.status = Some("この日に ToDo はありません".into());
                return;
            }
            self.focus = Focus::Detail;
            self.status = Some("タスクを選んで Space で状態を進めます".into());
            return;
        }
        let Some(line) = self.selected_note_task() else {
            self.status = Some("この日に ToDo はありません".into());
            return;
        };
        let note_idx = self.note_sel;
        // 行から今の状態とタイトルを引く。書き戻しの相手を探すのに使う。
        let Some(item) = self.notes[note_idx]
            .todo_items()
            .into_iter()
            .find(|i| i.line == line)
        else {
            return;
        };
        let next = item.status.next();

        let before = self.notes[note_idx].to_text();
        if !self.notes[note_idx].set_task_status(line, next) {
            self.status = Some("チェック欄が見つかりません".into());
            return;
        }
        if let Err(err) = self.store.save_note(&self.notes[note_idx]) {
            let path = self.notes[note_idx].path.clone();
            let date = self.notes[note_idx].date.clone();
            self.notes[note_idx] = DailyNote::parse(date, path, &before);
            self.status = Some(format!("保存に失敗しました: {err}"));
            return;
        }
        self.rebuild_todos();
        self.status = Some(format!("{} → {}", item.title, next.label()));
        if self.sync_projects_from_rows(&[(item.group.clone(), item.title.clone(), next)]) > 0 {
            self.status = Some(format!(
                "{} → {}（{} にも反映）",
                item.title,
                next.label(),
                item.group
            ));
        }
    }

    /// 選んでいるタスク行が画面に入るようスクロールを合わせる。
    fn scroll_to_note_task(&mut self) {
        let tasks = self.note_task_positions();
        let Some(at) = tasks.get(self.note_task_sel).copied() else {
            return;
        };
        let height = self.viewport.max(1);
        if at < self.detail_scroll {
            self.detail_scroll = at;
        } else if at >= self.detail_scroll + height {
            self.detail_scroll = at + 1 - height;
        }
    }

    /// 本文で選んでいるタスク行のファイル行番号。
    pub fn selected_note_task(&self) -> Option<usize> {
        let lines = self.detail_lines();
        let at = self
            .note_task_positions()
            .get(self.note_task_sel)
            .copied()?;
        lines.get(at)?.task_line
    }

    /// 本文ペインの中で、選べるタスク行が何行目にあるか。
    pub fn note_task_positions(&self) -> Vec<usize> {
        self.detail_lines()
            .iter()
            .enumerate()
            .filter(|(_, l)| l.task_line.is_some())
            .map(|(at, _)| at)
            .collect()
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
        for status in crate::model::STATUS_ORDER {
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

/// `2026-02-26 11:25` から時刻部分だけ取る。
///
/// Acta が書いたその日の最初のエントリは日付だけなので、時刻が無いこともある。
fn short_time(created: &str) -> Option<String> {
    created.split_once(' ').map(|(_, time)| time.to_string())
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
        assert_eq!(short_time("2026-02-26 11:25").as_deref(), Some("11:25"));
        assert_eq!(short_time("2026-02-26"), None);
    }
}
