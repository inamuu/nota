//! データディレクトリの走査と読み書き。

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

use crate::model::{initial_note_text, DailyNote, Project, TaskStatus};

const POSTS_DIR: &str = "posts";
const PROJECTS_DIR: &str = "projects";
const PROJECT_FILE: &str = "project.json";
const KNOWLEDGE_FILE: &str = "knowledge.md";
/// Acta の走査除外と揃える。
const IGNORED_DIRS: [&str; 6] = [".git", "node_modules", "dist", "release", "wiki", "images"];

pub struct Store {
    data_dir: PathBuf,
}

impl Store {
    pub fn new(data_dir: PathBuf) -> Self {
        Self { data_dir }
    }

    /// デイリーノートを新しい順に読み込む。
    pub fn load_notes(&self) -> Result<Vec<DailyNote>> {
        let mut paths = Vec::new();
        collect_date_files(&self.data_dir.join(POSTS_DIR), &mut paths);
        // ファイル名が YYYY-MM-DD なので文字列の降順が日付の降順になる。
        paths.sort_by(|a, b| b.file_name().cmp(&a.file_name()));

        let mut notes = Vec::with_capacity(paths.len());
        for path in paths {
            let Some(date) = date_from_path(&path) else {
                continue;
            };
            let text = std::fs::read_to_string(&path)
                .with_context(|| format!("読み込みに失敗しました: {}", path.display()))?;
            notes.push(DailyNote::parse(date, path, &text));
        }
        Ok(notes)
    }

    /// プロジェクトを更新の新しい順に読み込む。
    pub fn load_projects(&self) -> Result<Vec<Project>> {
        let dir = self.data_dir.join(PROJECTS_DIR);
        let Ok(read) = std::fs::read_dir(&dir) else {
            return Ok(Vec::new());
        };

        let mut out = Vec::new();
        for entry in read.flatten() {
            if !entry.path().is_dir() {
                continue;
            }
            let json_path = entry.path().join(PROJECT_FILE);
            let Ok(text) = std::fs::read_to_string(&json_path) else {
                continue;
            };
            // 1 件壊れていても全体を落とさない。
            let Ok(mut project) = serde_json::from_str::<Project>(&text) else {
                continue;
            };
            if project.name.is_empty() {
                project.name = entry.file_name().to_string_lossy().to_string();
            }
            project.knowledge =
                std::fs::read_to_string(entry.path().join(KNOWLEDGE_FILE)).unwrap_or_default();
            project.source_dir = entry.path();
            out.push(project);
        }
        out.sort_by_key(|p| std::cmp::Reverse(p.updated_at_ms));
        Ok(out)
    }

    /// ノートを書き戻す。同じディレクトリの一時ファイル経由で置き換え、
    /// 書き込み中に落ちても元ファイルが壊れないようにする。
    pub fn save_note(&self, note: &DailyNote) -> Result<()> {
        let text = note.to_text();
        let tmp = note.path.with_extension("md.nota-tmp");
        std::fs::write(&tmp, &text)
            .with_context(|| format!("書き込みに失敗しました: {}", tmp.display()))?;
        std::fs::rename(&tmp, &note.path)
            .with_context(|| format!("置き換えに失敗しました: {}", note.path.display()))?;
        Ok(())
    }
}

impl Store {
    /// その日のノートのパス。Acta と同じ `posts/YYYY/MM/DD/YYYY-MM-DD.md`。
    pub fn note_path(&self, date: &str) -> Option<PathBuf> {
        if !is_date(date) {
            return None;
        }
        let (year, rest) = date.split_at(4);
        let month = &rest[1..3];
        let day = &rest[4..6];
        Some(
            self.data_dir
                .join(POSTS_DIR)
                .join(year)
                .join(month)
                .join(day)
                .join(format!("{date}.md")),
        )
    }

    /// その日のノートを読む。無ければ見出しだけの中身で作って返す。
    /// 実際にファイルを作るのは保存のときなので、ここではディスクに触らない。
    pub fn load_or_create_note(&self, date: &str) -> Result<DailyNote> {
        let path = self
            .note_path(date)
            .with_context(|| format!("日付の形式が不正です: {date}"))?;
        let text = match std::fs::read_to_string(&path) {
            Ok(text) => text,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => initial_note_text(date),
            Err(err) => {
                return Err(err)
                    .with_context(|| format!("読み込みに失敗しました: {}", path.display()))
            }
        };
        Ok(DailyNote::parse(date.to_string(), path, &text))
    }

    /// ノートを保存する。親ディレクトリが無ければ作る。
    pub fn save_new_note(&self, note: &DailyNote) -> Result<()> {
        if let Some(parent) = note.path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("ディレクトリを作れません: {}", parent.display()))?;
        }
        self.save_note(note)
    }

    /// プロジェクトのタスク一覧を差し替える。
    ///
    /// JSON を丸ごと組み立て直すのではなく、読んだ `Value` の tasks だけを入れ替える。
    /// nota が解釈しない項目（`sourceType` や `sourceState` など）を落とさないため。
    /// タイトルが一致する既存タスクは、その項目ごと引き継ぐ。
    pub fn save_project_tasks(
        &self,
        project: &Project,
        tasks: &[(TaskStatus, String)],
        now_ms: i64,
    ) -> Result<()> {
        let path = project.source_dir.join(PROJECT_FILE);
        let text = std::fs::read_to_string(&path)
            .with_context(|| format!("読み込みに失敗しました: {}", path.display()))?;
        let mut root: serde_json::Value = serde_json::from_str(&text)
            .with_context(|| format!("JSON として読めません: {}", path.display()))?;

        // 既存タスクをタイトルで引ける形にする。同じタイトルが複数あれば順に使う。
        let mut existing: Vec<serde_json::Value> = root
            .get("tasks")
            .and_then(|t| t.as_array())
            .cloned()
            .unwrap_or_default();

        let mut next = Vec::with_capacity(tasks.len());
        for (status, title) in tasks {
            let found = existing
                .iter()
                .position(|t| t.get("title").and_then(|v| v.as_str()) == Some(title.as_str()));
            let mut task = match found {
                Some(at) => existing.remove(at),
                None => new_task_value(title, now_ms),
            };
            apply_status(&mut task, *status, now_ms);
            next.push(task);
        }

        root["tasks"] = serde_json::Value::Array(next);
        root["updatedAtMs"] = serde_json::Value::from(now_ms);

        // Acta と同じ 2 スペース、末尾の改行なし。
        let out = serde_json::to_string_pretty(&root).context("JSON を書き出せません")?;
        let tmp = path.with_extension("json.nota-tmp");
        std::fs::write(&tmp, &out)
            .with_context(|| format!("書き込みに失敗しました: {}", tmp.display()))?;
        std::fs::rename(&tmp, &path)
            .with_context(|| format!("置き換えに失敗しました: {}", path.display()))?;
        Ok(())
    }

    /// そのファイルが既にあるか。Acta は初日のエントリだけ created を日付のみにする。
    pub fn note_exists(&self, date: &str) -> bool {
        self.note_path(date).map(|p| p.is_file()).unwrap_or(false)
    }
}

/// 新しいタスクの JSON。Acta が作るものと同じ項目を埋める。
fn new_task_value(title: &str, now_ms: i64) -> serde_json::Value {
    serde_json::json!({
        "id": uuid::Uuid::new_v4().to_string(),
        "title": title,
        "status": "Backlog",
        "createdAtMs": now_ms,
        "updatedAtMs": now_ms,
        "completedAtMs": 0,
        "source": "local",
        "sourceUrl": "",
        "repository": "",
    })
}

/// 状態を書き込む。変わったときだけ updatedAtMs を動かし、
/// 完了になった時点で completedAtMs を記録する。
fn apply_status(task: &mut serde_json::Value, status: TaskStatus, now_ms: i64) {
    let before = task.get("status").and_then(|v| v.as_str()).unwrap_or("");
    let after = status.name();
    if before == after {
        return;
    }
    task["status"] = serde_json::Value::from(after);
    task["updatedAtMs"] = serde_json::Value::from(now_ms);
    match status {
        TaskStatus::Done => task["completedAtMs"] = serde_json::Value::from(now_ms),
        // 完了から戻したら記録も消す。Acta の表示と食い違わせない。
        _ => task["completedAtMs"] = serde_json::Value::from(0),
    }
}

/// `YYYY-MM-DD.md` だけを集める。slug 付きの記事ファイルは対象外。
fn collect_date_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(read) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in read.flatten() {
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().to_string();
        if path.is_dir() {
            if IGNORED_DIRS.contains(&name.as_str()) {
                continue;
            }
            collect_date_files(&path, out);
        } else if is_date_file(&name) {
            out.push(path);
        }
    }
}

fn is_date_file(name: &str) -> bool {
    let Some(stem) = name.strip_suffix(".md") else {
        return false;
    };
    is_date(stem)
}

fn is_date(s: &str) -> bool {
    let bytes = s.as_bytes();
    if bytes.len() != 10 {
        return false;
    }
    bytes.iter().enumerate().all(|(i, b)| match i {
        4 | 7 => *b == b'-',
        _ => b.is_ascii_digit(),
    })
}

fn date_from_path(path: &Path) -> Option<String> {
    let stem = path.file_stem()?.to_str()?;
    is_date(stem).then(|| stem.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::TaskStatus;

    /// テスト用の一時ディレクトリ。呼ぶたびに別の場所を使う。
    fn temp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("nota-test-{}-{tag}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("一時ディレクトリを作れる");
        dir
    }

    const NOTE: &str = "# 2026-01-01\n\n<!-- acta:comment\nid: t1\ncreated: 2026-01-01 09:00\ncreated_ms: 1\ntags: ToDo\n-->\n# ToDo: 2026/01/01（木）\n- プロジェクト\n  - [ ] やること\n  - [x] 済んだこと\n<!-- /acta:comment -->\n\n";

    fn seed(dir: &Path) {
        let post = dir.join("posts/2026/01/01");
        std::fs::create_dir_all(&post).expect("posts を作れる");
        std::fs::write(post.join("2026-01-01.md"), NOTE).expect("書ける");
    }

    /// チェックを進めて保存したとき、対象行の 1 文字だけが変わる。
    #[test]
    fn saving_changes_only_the_marker() {
        let dir = temp_dir("save");
        seed(&dir);
        let store = Store::new(dir.clone());

        let mut notes = store.load_notes().expect("読める");
        assert_eq!(notes.len(), 1);
        let items = notes[0].todo_items();
        assert_eq!(items.len(), 2);

        let line = items[0].line;
        assert!(notes[0].set_task_status(line, TaskStatus::InProgress));
        store.save_note(&notes[0]).expect("保存できる");

        let after =
            std::fs::read_to_string(dir.join("posts/2026/01/01/2026-01-01.md")).expect("読める");
        assert_eq!(after, NOTE.replace("- [ ] やること", "- [-] やること"));
        // 末尾の空行も保たれる。
        assert!(after.ends_with("<!-- /acta:comment -->\n\n"));

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 保存後に読み直しても同じ状態が得られる。
    #[test]
    fn saved_state_survives_reload() {
        let dir = temp_dir("reload");
        seed(&dir);
        let store = Store::new(dir.clone());

        let mut notes = store.load_notes().expect("読める");
        let line = notes[0].todo_items()[0].line;
        notes[0].set_task_status(line, TaskStatus::Done);
        store.save_note(&notes[0]).expect("保存できる");

        let reloaded = store.load_notes().expect("読める");
        let items = reloaded[0].todo_items();
        assert_eq!(items[0].status, TaskStatus::Done);
        assert_eq!(items[0].title, "やること");
        assert_eq!(items[1].status, TaskStatus::Done);

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 一時ファイルが残らない。残ると次回の走査で拾ってしまう。
    #[test]
    fn leaves_no_temporary_file() {
        let dir = temp_dir("tmp");
        seed(&dir);
        let store = Store::new(dir.clone());
        let notes = store.load_notes().expect("読める");
        store.save_note(&notes[0]).expect("保存できる");

        let post_dir = dir.join("posts/2026/01/01");
        let names: Vec<String> = std::fs::read_dir(&post_dir)
            .expect("読める")
            .flatten()
            .map(|e| e.file_name().to_string_lossy().to_string())
            .collect();
        assert_eq!(names, vec!["2026-01-01.md"]);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn accepts_canonical_date_file() {
        assert!(is_date_file("2026-02-26.md"));
    }

    #[test]
    fn rejects_slug_and_other_files() {
        assert!(!is_date_file("2026-07-14_some-slug.md"));
        assert!(!is_date_file("knowledge.md"));
        assert!(!is_date_file("2026-02-26.txt"));
        assert!(!is_date_file("2026-02-2.md"));
        assert!(!is_date_file("20260226.md"));
    }

    #[test]
    fn extracts_date_from_path() {
        assert_eq!(
            date_from_path(Path::new("/a/posts/2026/02/26/2026-02-26.md")).as_deref(),
            Some("2026-02-26")
        );
        assert_eq!(date_from_path(Path::new("/a/knowledge.md")), None);
    }
}
