//! Acta のデータ構造。
//!
//! 設計上の要点は「原文を捨てない」こと。`DailyNote` は読み込んだ行をそのまま保持し、
//! パース結果は行番号の範囲でそこを指す。書き戻しは該当行を差し替えて全体を再結合する
//! だけなので、空行・記法の揺れ・nota が解釈しない行がそのまま残る。
//! 編集機能を足すときもこの性質を崩さない。

use std::ops::Range;
use std::path::PathBuf;

use serde::Deserialize;

const OPEN_MARKER: &str = "<!-- acta:comment";
const CLOSE_MARKER: &str = "<!-- /acta:comment -->";
const META_END: &str = "-->";
pub const TODO_TAG: &str = "ToDo";
/// ToDo のタスク行のインデント。Acta と同じ半角 2 つ。
const TODO_NESTED_INDENT: &str = "  ";

/// デイリーノート 1 ファイル。
#[derive(Debug, Clone)]
pub struct DailyNote {
    pub date: String,
    pub path: PathBuf,
    /// 原文の行。末尾に改行があれば最後の要素は空文字列になる。
    /// 改行で分割したものをそのまま持つので、`join` するだけで原文に戻る。
    pub lines: Vec<String>,
    pub entries: Vec<Entry>,
}

/// `<!-- acta:comment ... -->` ブロック 1 件。
#[derive(Debug, Clone)]
#[allow(dead_code)] // 今の画面が読まない項目も書き戻しのため保持する
pub struct Entry {
    pub id: String,
    pub created: String,
    pub created_ms: i64,
    pub tags: Vec<String>,
    /// 本文が占める行範囲。`DailyNote::lines` へのインデックス。
    pub body: Range<usize>,
    /// 開きマーカーから閉じマーカーまでの行範囲。削除時に使う。
    pub block: Range<usize>,
}

/// ToDo エントリの中の 1 タスク行。
#[derive(Debug, Clone)]
#[allow(dead_code)] // 今の画面が読まない項目も書き戻しのため保持する
pub struct TodoItem {
    pub date: String,
    pub note_path: PathBuf,
    /// `DailyNote::lines` の絶対インデックス。書き換え対象。
    pub line: usize,
    pub group: String,
    pub title: String,
    pub status: TaskStatus,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TaskStatus {
    Backlog,
    InProgress,
    Done,
}

impl TaskStatus {
    pub fn marker(self) -> char {
        match self {
            Self::Backlog => ' ',
            Self::InProgress => '-',
            Self::Done => 'x',
        }
    }

    pub fn from_marker(c: char) -> Option<Self> {
        match c {
            ' ' => Some(Self::Backlog),
            '-' => Some(Self::InProgress),
            'x' | 'X' => Some(Self::Done),
            _ => None,
        }
    }

    /// Space キーで回す順序。Acta の 3 状態をそのまま巡回する。
    pub fn next(self) -> Self {
        match self {
            Self::Backlog => Self::InProgress,
            Self::InProgress => Self::Done,
            Self::Done => Self::Backlog,
        }
    }

    /// project.json に書く名前。Acta の表記と揃える。
    pub fn name(self) -> &'static str {
        match self {
            Self::Backlog => "Backlog",
            Self::InProgress => "InProgress",
            Self::Done => "Done",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Backlog => "Backlog",
            Self::InProgress => "InProgress",
            Self::Done => "Done",
        }
    }
}

impl<'de> Deserialize<'de> for TaskStatus {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let raw = String::deserialize(d)?;
        Ok(match raw.as_str() {
            "InProgress" => Self::InProgress,
            "Done" => Self::Done,
            _ => Self::Backlog,
        })
    }
}

/// `projects/<dir>/project.json`。
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
#[allow(dead_code)] // 今の画面が読まない項目も書き戻しのため保持する
pub struct Project {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub dir_name: String,
    #[serde(default)]
    pub created_at_ms: i64,
    #[serde(default)]
    pub updated_at_ms: i64,
    #[serde(default)]
    pub archived_at_ms: i64,
    #[serde(default)]
    pub issue_url: String,
    #[serde(default)]
    pub tasks: Vec<ProjectTask>,
    /// knowledge.md の中身。読み取り専用で保持する。
    #[serde(skip)]
    pub knowledge: String,
    /// このプロジェクトのディレクトリ。書き戻し先。
    #[serde(skip)]
    pub source_dir: PathBuf,
}

/// タスクを並べる順。Backlog を先頭に置く。まだ進んでいないものが
/// 上に来ないと、次に何をするかを探しにくい。
pub const STATUS_ORDER: [TaskStatus; 3] = [
    TaskStatus::Backlog,
    TaskStatus::InProgress,
    TaskStatus::Done,
];

impl Project {
    pub fn is_archived(&self) -> bool {
        self.archived_at_ms > 0
    }

    pub fn count(&self, status: TaskStatus) -> usize {
        self.tasks.iter().filter(|t| t.status == status).count()
    }

    /// タスクを状態別に取り出す。並びは元のまま。
    pub fn tasks_with(&self, status: TaskStatus) -> Vec<&ProjectTask> {
        self.tasks.iter().filter(|t| t.status == status).collect()
    }

    /// タスク一覧をチェックリストにする。これをエディタで開いて編集する。
    pub fn tasks_as_checklist(&self) -> String {
        let mut out = String::new();
        // 画面と同じ並びで出す。保存すると JSON もこの順になるので、
        // 編集の前後で一覧の並びが動かない。
        for status in STATUS_ORDER {
            for task in self.tasks_with(status) {
                out.push_str(&format!("- [{}] {}\n", status.marker(), task.title));
            }
        }
        out
    }
}

/// チェックリストを (状態, タイトル) の並びに戻す。
///
/// `- [ ] タイトル` を基本とするが、記法を知らずに名前だけ並べても拾えるよう、
/// 箇条書きの印もチェック欄も無い行は未着手のタスクとして扱う。
/// 見出しと空行だけは読み飛ばす。
pub fn parse_checklist(text: &str) -> Vec<(TaskStatus, String)> {
    text.lines()
        .filter_map(|line| {
            if let Some((status, title)) = parse_task_line(line) {
                return (!title.is_empty()).then_some((status, title));
            }
            let trimmed = line.trim();
            if trimmed.is_empty() || trimmed.starts_with('#') {
                return None;
            }
            // 箇条書きの印だけ付いている行も拾う。
            let title = trimmed
                .strip_prefix("- ")
                .or_else(|| trimmed.strip_prefix("* "))
                .unwrap_or(trimmed)
                .trim();
            (!title.is_empty()).then(|| (TaskStatus::Backlog, title.to_string()))
        })
        .collect()
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
#[allow(dead_code)] // 今の画面が読まない項目も書き戻しのため保持する
pub struct ProjectTask {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub title: String,
    #[serde(default = "default_status")]
    pub status: TaskStatus,
    #[serde(default)]
    pub updated_at_ms: i64,
}

fn default_status() -> TaskStatus {
    TaskStatus::Backlog
}

impl DailyNote {
    /// ファイル内容をパースする。解釈できない行も `lines` に残る。
    pub fn parse(date: String, path: PathBuf, text: &str) -> Self {
        let normalized = normalize(text);
        // 末尾の空行を数えずに済むよう、分割結果をそのまま持つ。
        // "a\n" は ["a", ""] になり、join で "a\n" に戻る。
        let lines: Vec<String> = normalized.split('\n').map(str::to_string).collect();
        let entries = parse_entries(&lines, &date);
        Self {
            date,
            path,
            lines,
            entries,
        }
    }

    /// 原文を復元する。パースしてそのまま呼べば元の内容と一致する。
    pub fn to_text(&self) -> String {
        self.lines.join("\n")
    }

    pub fn body_of(&self, entry: &Entry) -> Vec<&str> {
        self.lines[entry.body.clone()]
            .iter()
            .map(String::as_str)
            .collect()
    }

    /// ToDo として扱うエントリか。tags に ToDo、または本文が `# ToDo` 始まり。
    pub fn is_todo(&self, entry: &Entry) -> bool {
        if entry.tags.iter().any(|t| t.eq_ignore_ascii_case(TODO_TAG)) {
            return true;
        }
        self.lines[entry.body.clone()].iter().any(|line| {
            let t = line.trim_start();
            t.starts_with('#')
                && t.trim_start_matches('#')
                    .trim_start()
                    .to_lowercase()
                    .starts_with("todo")
        })
    }

    /// ToDo エントリからタスク行を抜き出す。行番号は絶対値。
    pub fn todo_items(&self) -> Vec<TodoItem> {
        let mut out = Vec::new();
        for entry in &self.entries {
            if !self.is_todo(entry) {
                continue;
            }
            let mut group = String::new();
            for idx in entry.body.clone() {
                let line = &self.lines[idx];
                if let Some((status, title)) = parse_task_line(line) {
                    out.push(TodoItem {
                        date: self.date.clone(),
                        note_path: self.path.clone(),
                        line: idx,
                        group: group.clone(),
                        title,
                        status,
                    });
                } else if let Some(name) = parse_group_line(line) {
                    group = name;
                }
            }
        }
        out
    }

    /// エントリ 1 件の中のタスク行を (グループ, タイトル, 状態) で返す。
    ///
    /// 編集の前後で見比べて、増えた行と消えた行をプロジェクトへ送るのに使う。
    /// 行番号は編集で動くので、ここでは持たない。
    pub fn entry_todo_rows(&self, entry_idx: usize) -> Vec<(String, String, TaskStatus)> {
        let Some(entry) = self.entries.get(entry_idx) else {
            return Vec::new();
        };
        if !self.is_todo(entry) {
            return Vec::new();
        }
        let mut group = String::new();
        let mut out = Vec::new();
        for idx in entry.body.clone() {
            let line = &self.lines[idx];
            if let Some((status, title)) = parse_task_line(line) {
                if !title.is_empty() {
                    out.push((group.clone(), title, status));
                }
            } else if let Some(name) = parse_group_line(line) {
                group = name;
            }
        }
        out
    }

    /// 行を書き換えたあとにエントリの行範囲を作り直す。
    /// 編集メソッドは必ずこれを通すので、範囲が古いままになることがない。
    fn reparse(&mut self) {
        self.entries = parse_entries(&self.lines, &self.date);
    }

    /// エントリの本文を差し替える。メタ行とマーカーには触らない。
    pub fn replace_entry_body(&mut self, entry_idx: usize, body: &str) -> bool {
        let Some(entry) = self.entries.get(entry_idx) else {
            return false;
        };
        let range = entry.body.clone();
        let replacement: Vec<String> = normalize(body)
            .trim_end_matches('\n')
            .split('\n')
            .map(str::to_string)
            .collect();
        self.lines.splice(range, replacement);
        self.reparse();
        true
    }

    /// メタ行の tags を差し替える。行が無ければメタ行の末尾に足す。
    pub fn set_entry_tags(&mut self, entry_idx: usize, tags: &[String]) -> bool {
        let Some(entry) = self.entries.get(entry_idx) else {
            return false;
        };
        // メタ行は開きマーカーの次から `-->` の直前まで。
        let start = entry.block.start + 1;
        let end = entry.body.start.saturating_sub(1);
        if start > end {
            return false;
        }
        let line = format!("tags: {}", tags.join(", "));
        match (start..end).find(|i| self.lines[*i].trim_start().starts_with("tags:")) {
            Some(at) => self.lines[at] = line,
            None => self.lines.insert(end, line),
        }
        self.reparse();
        true
    }

    /// エントリを削除する。Acta と同じく、続く空行もまとめて落とす。
    pub fn delete_entry(&mut self, entry_idx: usize) -> bool {
        let Some(entry) = self.entries.get(entry_idx) else {
            return false;
        };
        let start = entry.block.start;
        let mut end = entry.block.end;
        while end < self.lines.len() && self.lines[end].trim().is_empty() {
            end += 1;
        }
        // 末尾のエントリを消したときにファイルが改行なしで終わらないようにする。
        let at_end = end >= self.lines.len();
        self.lines.drain(start..end);
        if at_end {
            self.lines.truncate(start);
            self.lines.push(String::new());
        }
        self.reparse();
        true
    }

    /// エントリブロックを末尾に足す。
    ///
    /// ブロックの区切りが必ず空行 1 つになるよう、追記前に末尾だけ整える。
    /// Acta が書いたファイルは空行 2 つで終わっているので、その場合は
    /// 何も変わらず GUI と同じ並びになる。
    pub fn append_entry_block(&mut self, block: &str) {
        let mut text = self.to_text();
        if !text.is_empty() {
            while !text.ends_with("\n\n") {
                text.push('\n');
            }
        }
        text.push_str(block);
        let lines: Vec<String> = normalize(&text).split('\n').map(str::to_string).collect();
        self.lines = lines;
        self.reparse();
    }

    /// タスク行のマーカーを差し替える。行の他の部分には触らない。
    pub fn set_task_status(&mut self, line: usize, status: TaskStatus) -> bool {
        let Some(text) = self.lines.get(line) else {
            return false;
        };
        let Some(pos) = marker_position(text) else {
            return false;
        };
        let mut chars: Vec<char> = text.chars().collect();
        chars[pos] = status.marker();
        self.lines[line] = chars.into_iter().collect();
        true
    }
}

fn normalize(text: &str) -> String {
    text.replace("\r\n", "\n").replace('\r', "\n")
}

/// Acta の formatEntryBlock と同じ形。末尾は空行 1 つ。
pub fn format_entry_block(
    id: &str,
    created: &str,
    created_ms: i64,
    tags: &[String],
    body: &str,
) -> String {
    let tag_line = tags
        .iter()
        .map(|t| t.trim().trim_start_matches(['#', '＃']).to_string())
        .filter(|t| !t.is_empty())
        .collect::<Vec<_>>()
        .join(", ");
    let clean_body = normalize(body).trim_end().to_string();
    format!(
        "<!-- acta:comment\nid: {id}\ncreated: {created}\ncreated_ms: {created_ms}\ntags: {tag_line}\n-->\n{clean_body}\n<!-- /acta:comment -->\n\n"
    )
}

const WEEKDAY_JA: [&str; 7] = ["日", "月", "火", "水", "木", "金", "土"];

/// ToDo エントリの見出し。Acta と同じ `ToDo: 2026/08/25（月）` の形。
pub fn todo_heading(date: chrono::NaiveDate) -> String {
    use chrono::Datelike;
    let weekday = WEEKDAY_JA[date.weekday().num_days_from_sunday() as usize];
    format!(
        "{TODO_TAG}: {:04}/{:02}/{:02}（{weekday}）",
        date.year(),
        date.month(),
        date.day()
    )
}

/// 進行中のタスクから ToDo の本文を組み立てる。Acta の
/// buildTodoBodyFromProjectGroups と同じ並び。
pub fn build_todo_body(date: chrono::NaiveDate, groups: &[(&str, Vec<&ProjectTask>)]) -> String {
    let mut lines = vec![format!("# {}", todo_heading(date))];
    for (name, tasks) in groups {
        if tasks.is_empty() {
            continue;
        }
        lines.push(format!("- {name}"));
        for task in tasks {
            lines.push(format!(
                "{TODO_NESTED_INDENT}- [{}] {}",
                task.status.marker(),
                task.title
            ));
        }
    }
    lines.join("\n")
}

/// ToDo 本文にプロジェクトのタスクを流し込む。
///
/// Acta の upsertProjectTasksInTodoBody と同じ考え方で、
/// - 同じタイトルの行があればチェック欄だけ更新する
/// - 無いものは `append` が立っているときだけ足す
/// - `dropped` に挙げた行はそのグループから消す
/// - それ以外で ToDo 側にしかない行は消さない（手で足した予定を守る）
///
/// 戻り値は (新しい本文の行, 足した件数, 状態を変えた件数, 消した件数)。
pub fn upsert_todo_group(
    lines: &[String],
    project_name: &str,
    tasks: &[(TaskStatus, String, bool)],
    dropped: &[String],
) -> (Vec<String>, usize, usize, usize) {
    let mut out: Vec<String> = lines.to_vec();
    // 先にプロジェクトから消えた行を落とす。残る行の位置がずれないよう、後ろから消す。
    let mut removed = 0;
    if !dropped.is_empty() {
        if let Some(group) = find_todo_group(&out, project_name) {
            let targets: Vec<usize> = (group.start + 1..group.end)
                .filter(|at| {
                    parse_task_line(&out[*at]).is_some_and(|(_, title)| dropped.contains(&title))
                })
                .collect();
            for at in targets.into_iter().rev() {
                out.remove(at);
                removed += 1;
            }
        }
    }
    let Some(group) = find_todo_group(&out, project_name) else {
        // グループごと無いので、足すものだけ並べて末尾に置く。
        let adding: Vec<&(TaskStatus, String, bool)> =
            tasks.iter().filter(|(_, _, append)| *append).collect();
        if adding.is_empty() {
            return (out, 0, 0, removed);
        }
        while out.last().is_some_and(|l| l.trim().is_empty()) {
            out.pop();
        }
        out.push(format!("- {project_name}"));
        for (status, title, _) in &adding {
            out.push(format!(
                "{TODO_NESTED_INDENT}- [{}] {title}",
                status.marker()
            ));
        }
        return (out, adding.len(), 0, removed);
    };

    // グループ内の既存タスクをタイトルで引けるようにする。
    let mut seen: Vec<(String, usize)> = Vec::new();
    for (offset, line) in out[group.start + 1..group.end].iter().enumerate() {
        if let Some((_, title)) = parse_task_line(line) {
            if !title.is_empty() && !seen.iter().any(|(t, _)| *t == title) {
                seen.push((title, group.start + 1 + offset));
            }
        }
    }

    let mut updated = 0;
    let mut appending = Vec::new();
    for (status, title, append) in tasks {
        match seen.iter().find(|(t, _)| t == title) {
            Some((_, at)) => {
                if let Some(pos) = marker_position(&out[*at]) {
                    let mut chars: Vec<char> = out[*at].chars().collect();
                    if chars[pos] != status.marker() {
                        chars[pos] = status.marker();
                        out[*at] = chars.into_iter().collect();
                        updated += 1;
                    }
                }
            }
            None if *append => appending.push((*status, title.clone())),
            None => {}
        }
    }

    let added = appending.len();
    if added > 0 {
        let rows: Vec<String> = appending
            .into_iter()
            .map(|(status, title)| format!("{TODO_NESTED_INDENT}- [{}] {title}", status.marker()))
            .collect();
        out.splice(group.end..group.end, rows);
    }
    (out, added, updated, removed)
}

/// `- プロジェクト名` の行と、その配下が続く範囲。
struct TodoGroup {
    start: usize,
    /// 次のグループが始まる位置（この手前までが配下）。
    end: usize,
}

fn find_todo_group(lines: &[String], project_name: &str) -> Option<TodoGroup> {
    let start = lines
        .iter()
        .position(|l| parse_group_line(l).as_deref() == Some(project_name))?;
    let mut end = start + 1;
    while end < lines.len() {
        // インデントの無い行が来たら次のグループか別の記述。
        let line = &lines[end];
        if !line.trim().is_empty() && !line.starts_with(char::is_whitespace) {
            break;
        }
        end += 1;
    }
    // 配下の末尾にある空行は範囲から外す。追記はタスク行の直後に入れたい。
    while end > start + 1 && lines[end - 1].trim().is_empty() {
        end -= 1;
    }
    Some(TodoGroup { start, end })
}

/// 1 日分のファイルの初期内容。Acta が新規作成するときと同じ。
pub fn initial_note_text(date: &str) -> String {
    format!("# {date}\n\n")
}

/// `- [x] タイトル` を (状態, タイトル) に分解する。インデントの有無は問わない。
fn parse_task_line(line: &str) -> Option<(TaskStatus, String)> {
    let trimmed = line.trim_start();
    let rest = trimmed
        .strip_prefix("- ")
        .or_else(|| trimmed.strip_prefix("* "))?;
    let rest = rest.strip_prefix('[')?;
    let mut chars = rest.chars();
    let marker = chars.next()?;
    let after = chars.as_str();
    let title = after.strip_prefix(']')?.trim();
    let status = TaskStatus::from_marker(marker)?;
    Some((status, title.to_string()))
}

/// `- プロジェクト名` のグループ行。チェックボックス行は除く。
fn parse_group_line(line: &str) -> Option<String> {
    if line.starts_with(char::is_whitespace) {
        return None;
    }
    let rest = line
        .strip_prefix("- ")
        .or_else(|| line.strip_prefix("* "))?
        .trim();
    if rest.starts_with('[') || rest.is_empty() {
        return None;
    }
    Some(rest.to_string())
}

/// 行内の `[x]` の x の位置を char 単位で返す。
fn marker_position(line: &str) -> Option<usize> {
    let chars: Vec<char> = line.chars().collect();
    for i in 0..chars.len().saturating_sub(2) {
        if chars[i] == '[' && chars[i + 2] == ']' && TaskStatus::from_marker(chars[i + 1]).is_some()
        {
            return Some(i + 1);
        }
    }
    None
}

fn parse_entries(lines: &[String], date: &str) -> Vec<Entry> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < lines.len() {
        let line = lines[i].trim();
        if !(line.starts_with(OPEN_MARKER) && !line.contains(META_END)) {
            i += 1;
            continue;
        }
        let block_start = i;
        // メタ行は開きマーカーの次から `-->` の直前まで。
        let mut j = i + 1;
        let mut meta_end = None;
        while j < lines.len() {
            if lines[j].trim() == META_END {
                meta_end = Some(j);
                break;
            }
            j += 1;
        }
        let Some(meta_end) = meta_end else {
            break;
        };
        let body_start = meta_end + 1;
        let mut k = body_start;
        let mut close = None;
        while k < lines.len() {
            if lines[k].trim() == CLOSE_MARKER {
                close = Some(k);
                break;
            }
            k += 1;
        }
        let Some(close) = close else {
            break;
        };

        let meta = parse_meta(&lines[i + 1..meta_end]);
        let created = meta
            .get("created")
            .cloned()
            .unwrap_or_else(|| date.to_string());
        let created_ms = meta
            .get("created_ms")
            .and_then(|v| v.parse::<i64>().ok())
            .unwrap_or(0);
        let id = meta
            .get("id")
            .cloned()
            .unwrap_or_else(|| format!("{date}:{block_start}"));
        let tags = meta.get("tags").map(|v| parse_tags(v)).unwrap_or_default();

        out.push(Entry {
            id,
            created,
            created_ms,
            tags,
            body: body_start..close,
            block: block_start..close + 1,
        });
        i = close + 1;
    }
    out
}

fn parse_meta(lines: &[String]) -> std::collections::HashMap<String, String> {
    let mut map = std::collections::HashMap::new();
    for line in lines {
        let Some((key, value)) = line.split_once(':') else {
            continue;
        };
        let key = key.trim();
        if key.is_empty() || !key.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
            continue;
        }
        map.insert(key.to_string(), value.trim().to_string());
    }
    map
}

/// Acta と同じ規則。カンマまたは読点区切り、先頭の # を落とし、重複を除く。
pub fn parse_tags(raw: &str) -> Vec<String> {
    let mut out = Vec::new();
    for part in raw.split([',', '、']) {
        let tag = part
            .trim()
            .trim_start_matches(['#', '＃'])
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ");
        if !tag.is_empty() && !out.contains(&tag) {
            out.push(tag);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "# 2026-02-26\n\
        \n\
        <!-- acta:comment\n\
        id: abc-123\n\
        created: 2026-02-26 11:25\n\
        created_ms: 1772072708000\n\
        tags: ToDo, Terraform\n\
        -->\n\
        # ToDo: 2026/02/26（木）\n\
        - SRE対応依頼\n\
        \x20 - [ ] 未着手のタスク\n\
        \x20 - [-] 進行中のタスク\n\
        \x20 - [x] 完了のタスク\n\
        <!-- /acta:comment -->\n\
        \n\
        <!-- acta:comment\n\
        id: def-456\n\
        created: 2026-02-26 12:19\n\
        created_ms: 1772075960974\n\
        tags: JM_QA移行\n\
        -->\n\
        ## なるジョブ CI\n\
        \n\
        - 本文\n\
        <!-- /acta:comment -->\n";

    fn sample() -> DailyNote {
        DailyNote::parse(
            "2026-02-26".to_string(),
            PathBuf::from("/tmp/2026-02-26.md"),
            SAMPLE,
        )
    }

    #[test]
    fn parses_all_entries() {
        let note = sample();
        assert_eq!(note.entries.len(), 2);
        assert_eq!(note.entries[0].id, "abc-123");
        assert_eq!(note.entries[0].created_ms, 1772072708000);
        assert_eq!(note.entries[1].tags, vec!["JM_QA移行"]);
    }

    #[test]
    fn keeps_body_lines() {
        let note = sample();
        let body = note.body_of(&note.entries[1]);
        assert_eq!(body, vec!["## なるジョブ CI", "", "- 本文"]);
    }

    /// 原文を壊さないことが編集機能の前提なので、往復を必ず検証する。
    #[test]
    fn round_trips_original_text() {
        let note = sample();
        assert_eq!(note.to_text(), SAMPLE);
    }

    #[test]
    fn detects_todo_entry() {
        let note = sample();
        assert!(note.is_todo(&note.entries[0]));
        assert!(!note.is_todo(&note.entries[1]));
    }

    #[test]
    fn extracts_todo_items_with_group() {
        let note = sample();
        let items = note.todo_items();
        assert_eq!(items.len(), 3);
        assert_eq!(items[0].group, "SRE対応依頼");
        assert_eq!(items[0].status, TaskStatus::Backlog);
        assert_eq!(items[1].status, TaskStatus::InProgress);
        assert_eq!(items[2].status, TaskStatus::Done);
        assert_eq!(items[0].title, "未着手のタスク");
    }

    #[test]
    fn toggling_status_only_touches_the_marker() {
        let mut note = sample();
        let items = note.todo_items();
        let line = items[0].line;
        let before = note.lines[line].clone();
        assert!(note.set_task_status(line, TaskStatus::Done));
        assert_eq!(note.lines[line], before.replace("[ ]", "[x]"));
        // 他の行は変わらない。
        assert_eq!(note.entries.len(), 2);
        assert_eq!(note.lines.len(), sample().lines.len());
    }

    #[test]
    fn status_cycles_through_three_states() {
        assert_eq!(TaskStatus::Backlog.next(), TaskStatus::InProgress);
        assert_eq!(TaskStatus::InProgress.next(), TaskStatus::Done);
        assert_eq!(TaskStatus::Done.next(), TaskStatus::Backlog);
    }

    #[test]
    fn parses_a_checklist() {
        let tasks = parse_checklist("- [-] 進行中\n- [ ] 未着手\n- [x] 完了\n");
        assert_eq!(
            tasks,
            vec![
                (TaskStatus::InProgress, "進行中".to_string()),
                (TaskStatus::Backlog, "未着手".to_string()),
                (TaskStatus::Done, "完了".to_string()),
            ]
        );
    }

    /// 記法を知らずに名前だけ並べても、未着手のタスクとして拾う。
    #[test]
    fn parses_plain_lines_as_tasks() {
        let tasks = parse_checklist("最初の作業\n- 次の作業\n\n# 見出しは飛ばす\n");
        assert_eq!(
            tasks,
            vec![
                (TaskStatus::Backlog, "最初の作業".to_string()),
                (TaskStatus::Backlog, "次の作業".to_string()),
            ]
        );
    }

    #[test]
    fn builds_the_todo_heading() {
        let date = chrono::NaiveDate::from_ymd_opt(2026, 8, 25).expect("日付");
        assert_eq!(todo_heading(date), "ToDo: 2026/08/25（火）");
    }

    #[test]
    fn builds_a_todo_body_from_tasks() {
        let date = chrono::NaiveDate::from_ymd_opt(2026, 8, 25).expect("日付");
        let task = ProjectTask {
            id: "x".into(),
            title: "進行中の作業".into(),
            status: TaskStatus::InProgress,
            updated_at_ms: 0,
        };
        let body = build_todo_body(date, &[("プロジェクト", vec![&task]), ("空", vec![])]);
        assert_eq!(
            body,
            "# ToDo: 2026/08/25（火）\n- プロジェクト\n  - [-] 進行中の作業"
        );
        // 読み直すとタスクとして拾える。
        let text = format!(
            "<!-- acta:comment\nid: x\ncreated: 2026-08-25\ncreated_ms: 1\ntags: ToDo\n-->\n{body}\n<!-- /acta:comment -->\n"
        );
        let note = DailyNote::parse("2026-08-25".into(), PathBuf::from("/tmp/x.md"), &text);
        let items = note.todo_items();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].group, "プロジェクト");
        assert_eq!(items[0].status, TaskStatus::InProgress);
    }

    #[test]
    fn builds_a_heading_only_body_without_tasks() {
        let date = chrono::NaiveDate::from_ymd_opt(2026, 8, 25).expect("日付");
        assert_eq!(build_todo_body(date, &[]), "# ToDo: 2026/08/25（火）");
    }

    fn to_lines(text: &str) -> Vec<String> {
        text.split('\n').map(str::to_string).collect()
    }

    const TODO_BODY: &str = "# ToDo: 2026/08/26（水）\n\
        - プロジェクト A\n\
        \x20 - [-] 進行中の作業\n\
        \x20 - [ ] 手で足した予定\n\
        - プロジェクト B\n\
        \x20 - [ ] 別の作業";

    #[test]
    fn adds_new_tasks_to_an_existing_group() {
        let (out, added, updated, _) = upsert_todo_group(
            &to_lines(TODO_BODY),
            "プロジェクト A",
            &[(TaskStatus::InProgress, "新しい作業".into(), true)],
            &[],
        );
        assert_eq!((added, updated), (1, 0));
        // グループの末尾に入り、次のグループは動かない。
        assert_eq!(
            out.join("\n"),
            "# ToDo: 2026/08/26（水）\n\
             - プロジェクト A\n\
             \x20 - [-] 進行中の作業\n\
             \x20 - [ ] 手で足した予定\n\
             \x20 - [-] 新しい作業\n\
             - プロジェクト B\n\
             \x20 - [ ] 別の作業"
        );
    }

    /// 同じタイトルの行はチェック欄だけ変える。二重に足さない。
    #[test]
    fn updates_the_marker_of_an_existing_task() {
        let (out, added, updated, _) = upsert_todo_group(
            &to_lines(TODO_BODY),
            "プロジェクト A",
            &[(TaskStatus::Done, "進行中の作業".into(), true)],
            &[],
        );
        assert_eq!((added, updated), (0, 1));
        assert!(out.join("\n").contains("  - [x] 進行中の作業"));
        assert_eq!(out.len(), to_lines(TODO_BODY).len(), "行が増えている");
    }

    /// ToDo 側にしか無い行は残す。手で足した予定を消さない。
    #[test]
    fn keeps_rows_that_only_exist_in_the_todo() {
        let (out, _, _, _) = upsert_todo_group(
            &to_lines(TODO_BODY),
            "プロジェクト A",
            &[(TaskStatus::InProgress, "進行中の作業".into(), true)],
            &[],
        );
        assert!(out.join("\n").contains("- [ ] 手で足した予定"));
    }

    /// append が false のものは、既存行の更新だけで新しくは足さない。
    #[test]
    fn does_not_append_when_not_requested() {
        let (out, added, _, _) = upsert_todo_group(
            &to_lines(TODO_BODY),
            "プロジェクト A",
            &[(TaskStatus::Done, "完了した別の作業".into(), false)],
            &[],
        );
        assert_eq!(added, 0);
        assert!(!out.join("\n").contains("完了した別の作業"));
    }

    /// グループが無ければ末尾に作る。
    #[test]
    fn creates_a_group_when_missing() {
        let (out, added, _, _) = upsert_todo_group(
            &to_lines(TODO_BODY),
            "新しいプロジェクト",
            &[(TaskStatus::InProgress, "作業".into(), true)],
            &[],
        );
        assert_eq!(added, 1);
        assert!(out
            .join("\n")
            .ends_with("- 新しいプロジェクト\n  - [-] 作業"));
    }

    /// 足すものが無ければグループも作らない。
    #[test]
    fn creates_nothing_when_there_is_nothing_to_add() {
        let before = to_lines(TODO_BODY);
        let (out, added, updated, _) = upsert_todo_group(
            &before,
            "新しいプロジェクト",
            &[(TaskStatus::Done, "完了".into(), false)],
            &[],
        );
        assert_eq!((added, updated), (0, 0));
        assert_eq!(out, before);
    }

    /// 見出しだけの ToDo にも足せる。
    #[test]
    fn adds_to_a_heading_only_todo() {
        let (out, added, _, _) = upsert_todo_group(
            &to_lines("# ToDo: 2026/08/26（水）"),
            "プロジェクト",
            &[(TaskStatus::InProgress, "作業".into(), true)],
            &[],
        );
        assert_eq!(added, 1);
        assert_eq!(
            out.join("\n"),
            "# ToDo: 2026/08/26（水）\n- プロジェクト\n  - [-] 作業"
        );
    }

    #[test]
    fn parses_tags_like_acta() {
        assert_eq!(parse_tags("#a, b、 c"), vec!["a", "b", "c"]);
        assert_eq!(parse_tags(" , ,"), Vec::<String>::new());
        assert_eq!(parse_tags("dup, dup"), vec!["dup"]);
    }

    #[test]
    fn ignores_unterminated_block() {
        let note = DailyNote::parse(
            "2026-01-01".to_string(),
            PathBuf::from("/tmp/x.md"),
            "<!-- acta:comment\nid: x\n-->\nbody without close\n",
        );
        assert!(note.entries.is_empty());
        // 原文は保持される（末尾改行の分を含めて 5 要素）。
        assert_eq!(note.lines.len(), 5);
        assert_eq!(
            note.to_text(),
            "<!-- acta:comment\nid: x\n-->\nbody without close\n"
        );
    }

    #[test]
    fn group_line_ignores_checkbox_rows() {
        assert_eq!(
            parse_group_line("- プロジェクト"),
            Some("プロジェクト".into())
        );
        assert_eq!(parse_group_line("  - [ ] タスク"), None);
        assert_eq!(parse_group_line("- [ ] タスク"), None);
    }

    #[test]
    fn replaces_only_the_target_body() {
        let mut note = sample();
        assert!(note.replace_entry_body(1, "書き換えた本文\n2 行目"));
        let text = note.to_text();
        // 対象の本文だけが変わる。
        assert!(text.contains("書き換えた本文\n2 行目\n<!-- /acta:comment -->"));
        assert!(!text.contains("## なるジョブ CI"));
        // メタ行と他のエントリは残る。
        assert!(text.contains("id: def-456"));
        assert!(text.contains("created_ms: 1772075960974"));
        assert!(text.contains("# ToDo: 2026/02/26（木）"));
        assert_eq!(note.entries.len(), 2);
        // 行範囲が作り直されている。
        assert_eq!(
            note.body_of(&note.entries[1]),
            vec!["書き換えた本文", "2 行目"]
        );
    }

    #[test]
    fn deletes_an_entry_with_its_blank_lines() {
        let mut note = sample();
        assert!(note.delete_entry(0));
        assert_eq!(note.entries.len(), 1);
        let text = note.to_text();
        assert!(!text.contains("abc-123"));
        assert!(text.contains("id: def-456"));
        // 見出しは残り、余分な空行も残らない。
        assert!(text.starts_with("# 2026-02-26\n\n<!-- acta:comment\nid: def-456"));
    }

    #[test]
    fn deleting_the_last_entry_keeps_a_trailing_newline() {
        let mut note = sample();
        assert!(note.delete_entry(1));
        assert_eq!(note.entries.len(), 1);
        let text = note.to_text();
        // Acta の形式ではブロックの後に空行が 1 つ入る。
        assert!(
            text.ends_with("<!-- /acta:comment -->\n\n"),
            "末尾が不正: {text:?}"
        );
    }

    /// GUI が追記した結果と同じ並びになる。
    /// 末尾が改行 1 つのファイルでも、ブロックの間に空行が 1 つ入る。
    #[test]
    fn appending_keeps_one_blank_line_between_blocks() {
        let mut note = sample();
        let block = format_entry_block(
            "new-id",
            "2026-02-26 15:00",
            1772000000000,
            &["Memo".to_string()],
            "新しい本文",
        );
        note.append_entry_block(&block);
        assert_eq!(note.entries.len(), 3);
        let text = note.to_text();
        assert!(text.contains("<!-- /acta:comment -->\n\n<!-- acta:comment\nid: new-id"));
        assert!(text.ends_with("新しい本文\n<!-- /acta:comment -->\n\n"));
        assert_eq!(note.entries[2].tags, vec!["Memo"]);
    }

    #[test]
    fn block_format_matches_acta() {
        let block = format_entry_block(
            "i",
            "2026-01-01 09:00",
            1,
            &["a".into(), "b".into()],
            "本文\n",
        );
        assert_eq!(
            block,
            "<!-- acta:comment\nid: i\ncreated: 2026-01-01 09:00\ncreated_ms: 1\ntags: a, b\n-->\n本文\n<!-- /acta:comment -->\n\n"
        );
    }

    #[test]
    fn block_format_allows_empty_tags() {
        let block = format_entry_block("i", "2026-01-01", 1, &[], "本文");
        assert!(block.contains("tags: \n"));
    }

    /// 追記したブロックを読み直しても壊れない。
    #[test]
    fn round_trips_after_appending() {
        let mut note = DailyNote::parse(
            "2026-01-01".into(),
            PathBuf::from("/tmp/x.md"),
            &initial_note_text("2026-01-01"),
        );
        assert!(note.entries.is_empty());
        note.append_entry_block(&format_entry_block("a", "2026-01-01", 1, &[], "一つ目"));
        note.append_entry_block(&format_entry_block(
            "b",
            "2026-01-01 10:00",
            2,
            &[],
            "二つ目",
        ));
        assert_eq!(note.entries.len(), 2);

        let text = note.to_text();
        let again = DailyNote::parse("2026-01-01".into(), PathBuf::from("/tmp/x.md"), &text);
        assert_eq!(again.to_text(), text);
        assert_eq!(again.entries.len(), 2);
        assert_eq!(again.body_of(&again.entries[0]), vec!["一つ目"]);
    }

    /// 実データで見つかった不具合の回帰テスト。
    /// 末尾に空行が続くファイルを書き戻すと、以前は空行が 1 つ消えていた。
    #[test]
    fn round_trips_trailing_blank_lines() {
        for text in ["a\n", "a\n\n", "a\n\n\n", "a", "\n", "\n\n"] {
            let note = DailyNote::parse("2026-01-01".into(), PathBuf::from("/tmp/x.md"), text);
            assert_eq!(note.to_text(), text, "往復に失敗: {text:?}");
        }
    }

    #[test]
    fn empty_file_round_trips() {
        let note = DailyNote::parse("2026-01-01".into(), PathBuf::from("/tmp/x.md"), "");
        assert_eq!(note.to_text(), "");
    }
}
