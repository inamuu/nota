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
}

impl Project {
    pub fn is_archived(&self) -> bool {
        self.archived_at_ms > 0
    }

    pub fn count(&self, status: TaskStatus) -> usize {
        self.tasks.iter().filter(|t| t.status == status).count()
    }
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
        let normalized = text.replace("\r\n", "\n").replace('\r', "\n");
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
