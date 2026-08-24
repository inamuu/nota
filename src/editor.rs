//! 外部エディタの起動。
//!
//! 渡すのはエントリの本文だけで、ファイル全体は開かせない。`acta:comment` の
//! メタ行や閉じマーカーを人が触れる余地をなくすためで、編集対象がそのまま
//! 画面に出るので目的の箇所を探す必要もない。

use std::path::PathBuf;
use std::process::Command;

use anyhow::{bail, Context, Result};

/// 何を編集しているか。エディタから戻ったときの適用先。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EditTarget {
    /// 既存エントリの本文を差し替える。
    EntryBody { note_idx: usize, entry_idx: usize },
    /// 今日のノートに新しいエントリを足す。
    NewEntry,
    /// プロジェクトのタスク一覧をチェックリストとして編集する。
    ProjectTasks { project_idx: usize },
}

#[derive(Debug, Clone)]
pub struct EditRequest {
    pub target: EditTarget,
    /// エディタに最初から入っている内容。
    pub initial: String,
}

/// エディタに渡す形。1 行目がタグで、空行のあとが本文。
///
/// メタ行そのものを見せると `created_ms` や閉じマーカーを壊せてしまうので、
/// 人が触ってよいタグだけを 1 行に出す。
pub fn compose(tags: &[String], body: &str) -> String {
    format!("tags: {}\n\n{}", tags.join(", "), body.trim_end())
}

/// エディタから戻ってきた内容を (タグ, 本文) に分ける。
///
/// 1 行目が `tags:` ならタグとして取り込み、続く空行 1 つを区切りとして落とす。
/// タグ行を消して保存したときは、タグなしの本文として扱う。
pub fn decompose(text: &str) -> (Vec<String>, String) {
    let normalized = text.replace("\r\n", "\n").replace('\r', "\n");
    let Some(rest) = normalized.strip_prefix("tags:") else {
        return (Vec::new(), normalized.trim_end().to_string());
    };
    let (tag_line, body) = match rest.split_once('\n') {
        Some((tag_line, body)) => (tag_line, body),
        None => (rest, ""),
    };
    let tags = crate::model::parse_tags(tag_line);
    // 区切りの空行は 1 つだけ落とす。本文側の空行は保つ。
    let body = body.strip_prefix('\n').unwrap_or(body);
    (tags, body.trim_end().to_string())
}

/// `$EDITOR` を起動して、編集後の内容を返す。
///
/// 端末の制御は呼び出し側が外しておくこと。内容が変わらなければ `None`。
pub fn run(initial: &str, tag: &str) -> Result<Option<String>> {
    run_with(&editor_command()?, initial, tag)
}

/// 起動の本体。エディタを引数で受けるので、テストから環境変数に触らず呼べる。
fn run_with(editor: &[String], initial: &str, tag: &str) -> Result<Option<String>> {
    let path = temp_path(tag);

    std::fs::write(&path, initial)
        .with_context(|| format!("一時ファイルを作れません: {}", path.display()))?;

    let status = spawn(editor, &path);
    // エディタが失敗しても一時ファイルは残さない。
    let result = match status {
        Ok(status) if status.success() => std::fs::read_to_string(&path)
            .with_context(|| format!("編集結果を読めません: {}", path.display())),
        Ok(status) => Err(anyhow::anyhow!(
            "エディタが異常終了しました（{}）: {}",
            status
                .code()
                .map(|c| c.to_string())
                .unwrap_or_else(|| "シグナル".into()),
            editor.join(" ")
        )),
        Err(err) => Err(err),
    };
    let _ = std::fs::remove_file(&path);

    let edited = result?;
    Ok(if edited == initial {
        None
    } else {
        Some(edited)
    })
}

/// `$VISUAL` / `$EDITOR` からエディタを決める。
fn editor_command() -> Result<Vec<String>> {
    parse_editor(
        std::env::var("VISUAL").ok().as_deref(),
        std::env::var("EDITOR").ok().as_deref(),
    )
}

/// 決定の本体。環境変数を読まないので、テストから直接呼べる。
///
/// テスト内で `set_var` を使うと、並列実行中の他スレッドの環境変数読み取りと
/// 競合するため、環境変数に触るのは上の 1 か所だけにしてある。
fn parse_editor(visual: Option<&str>, editor: Option<&str>) -> Result<Vec<String>> {
    // VISUAL を優先する。`nvim -u NONE` のように引数付きの指定も通す。
    for value in [visual, editor].into_iter().flatten() {
        let parts: Vec<String> = value.split_whitespace().map(str::to_string).collect();
        if !parts.is_empty() {
            return Ok(parts);
        }
    }
    bail!("エディタが設定されていません。EDITOR か VISUAL を設定してください（例: export EDITOR=nvim）")
}

fn spawn(editor: &[String], path: &PathBuf) -> Result<std::process::ExitStatus> {
    let (program, args) = editor.split_first().expect("空でない");
    Command::new(program)
        .args(args)
        .arg(path)
        .status()
        .with_context(|| format!("エディタを起動できません: {program}"))
}

/// 一時ファイルの名前。拡張子を .md にしておくとエディタ側で
/// Markdown として扱われる。
fn temp_path(tag: &str) -> PathBuf {
    let unique = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    std::env::temp_dir().join(format!("nota-{tag}-{}-{unique}.md", std::process::id()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn composes_tags_and_body() {
        assert_eq!(
            compose(&["a".into(), "b".into()], "本文\n2 行目\n"),
            "tags: a, b\n\n本文\n2 行目"
        );
        assert_eq!(compose(&[], "本文"), "tags: \n\n本文");
    }

    #[test]
    fn decomposes_tags_and_body() {
        let (tags, body) = decompose("tags: a, b\n\n本文\n2 行目\n");
        assert_eq!(tags, vec!["a", "b"]);
        assert_eq!(body, "本文\n2 行目");
    }

    /// タグ行を消して保存したらタグなしとして扱う。
    #[test]
    fn decomposes_without_a_tag_line() {
        let (tags, body) = decompose("本文だけ\n");
        assert!(tags.is_empty());
        assert_eq!(body, "本文だけ");
    }

    #[test]
    fn decomposes_empty_tag_line() {
        let (tags, body) = decompose("tags: \n\n本文");
        assert!(tags.is_empty());
        assert_eq!(body, "本文");
    }

    /// 本文の中の空行は保つ。区切りとして落とすのは 1 つだけ。
    #[test]
    fn keeps_blank_lines_inside_the_body() {
        let (_, body) = decompose("tags: x\n\n一段落\n\n二段落");
        assert_eq!(body, "一段落\n\n二段落");
    }

    /// 組み立てて分解すると元に戻る。
    #[test]
    fn round_trips_through_the_editor_format() {
        let tags = vec!["ToDo".to_string(), "Terraform".to_string()];
        let body = "# 見出し\n\n- 項目";
        let (back_tags, back_body) = decompose(&compose(&tags, body));
        assert_eq!(back_tags, tags);
        assert_eq!(back_body, body);
    }

    #[test]
    fn splits_editor_arguments() {
        assert_eq!(
            parse_editor(None, Some("nvim -u NONE")).expect("解決できる"),
            vec!["nvim", "-u", "NONE"]
        );
    }

    #[test]
    fn visual_wins_over_editor() {
        assert_eq!(
            parse_editor(Some("code --wait"), Some("vi")).expect("解決できる"),
            vec!["code", "--wait"]
        );
    }

    /// 空や空白だけの指定は「無い」として次を見る。
    #[test]
    fn blank_values_fall_through() {
        assert_eq!(
            parse_editor(Some("   "), Some("vi")).expect("解決できる"),
            vec!["vi"]
        );
        let err = parse_editor(Some(""), Some("  "))
            .expect_err("エラーになる")
            .to_string();
        assert!(err.contains("EDITOR"), "案内が出ていない: {err}");
        assert!(parse_editor(None, None).is_err());
    }

    #[test]
    fn temp_paths_are_unique() {
        let a = temp_path("x");
        let b = temp_path("x");
        assert_ne!(a, b);
        assert_eq!(a.extension().and_then(|e| e.to_str()), Some("md"));
    }

    /// 中身を書き換えないエディタなら None を返す。true は何もせず成功する。
    #[test]
    fn unchanged_content_reports_none() {
        let editor = vec!["true".to_string()];
        assert!(run_with(&editor, "そのまま", "unchanged")
            .expect("成功する")
            .is_none());
    }

    /// 書き換えられたら中身を返す。
    #[test]
    fn changed_content_is_returned() {
        // 一時ファイルを上書きするだけのエディタ代わり。
        let editor = vec![
            "sh".to_string(),
            "-c".to_string(),
            "echo 書き換え > \"$1\"".to_string(),
            "sh".to_string(),
        ];
        let result = run_with(&editor, "もとの本文", "changed").expect("成功する");
        assert_eq!(result.as_deref().map(str::trim), Some("書き換え"));
    }

    /// 異常終了はエラーとして扱い、編集結果を採用しない。
    #[test]
    fn failing_editor_is_an_error() {
        let editor = vec!["false".to_string()];
        assert!(run_with(&editor, "本文", "failing").is_err());
    }

    /// 起動できないコマンドもエラーで返す。パニックさせない。
    #[test]
    fn missing_editor_is_an_error() {
        let editor = vec!["nota-no-such-editor".to_string()];
        assert!(run_with(&editor, "本文", "missing").is_err());
    }

    /// 一時ファイルを残さない。エディタが失敗した場合も消す。
    #[test]
    fn removes_the_temporary_file() {
        let before = count_temp_files();
        let _ = run_with(&["true".to_string()], "x", "cleanup");
        let _ = run_with(&["false".to_string()], "x", "cleanup");
        assert_eq!(count_temp_files(), before, "一時ファイルが残っている");
    }

    fn count_temp_files() -> usize {
        std::fs::read_dir(std::env::temp_dir())
            .map(|dir| {
                dir.flatten()
                    .filter(|e| {
                        e.file_name()
                            .to_string_lossy()
                            .starts_with(&format!("nota-cleanup-{}", std::process::id()))
                    })
                    .count()
            })
            .unwrap_or(0)
    }
}
