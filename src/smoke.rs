//! 描画と実データ読み込みの検証。
//!
//! TUI は目で見ないと分からない部分が多いので、少なくとも「落ちない」ことと
//! 「実データを解釈できる」ことは自動で確かめる。

#![cfg(test)]

use std::path::{Path, PathBuf};

use ratatui::backend::TestBackend;
use ratatui::Terminal;

use crate::app::{App, Confirm, Focus, Mode, Move, Msg, TodoSort, View};
use crate::config::Config;
use crate::editor::EditTarget;
use crate::model::TaskStatus;

fn empty_app() -> App {
    let mut app = App::new(Config {
        data_dir: PathBuf::from("/nonexistent"),
        source: "test".into(),
        recent_notes: 30,
        project_done_limit: 5,
    })
    .expect("データが無くても起動できる");
    // 起動時のロゴは邪魔なので閉じる。ロゴ自体は専用のテストで見る。
    app.dismiss_splash();
    app
}

fn render(app: &mut App, width: u16, height: u16) -> String {
    let mut terminal = Terminal::new(TestBackend::new(width, height)).expect("terminal");
    terminal
        .draw(|frame| crate::ui::draw(app, frame))
        .expect("draw");
    let buffer = terminal.backend().buffer().clone();
    (0..buffer.area.height)
        .map(|y| {
            (0..buffer.area.width)
                .map(|x| buffer[(x, y)].symbol().to_string())
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// 画面から空白を落とす。TestBackend では全角文字の 2 セル目がスペースで埋まるため、
/// 文字列の一致を見るときはこれを通す。
fn squash(screen: &str) -> String {
    screen.chars().filter(|c| !c.is_whitespace()).collect()
}

/// 日付だけ違うノートを count 件持つ一時データを作る。
fn seeded_app(tag: &str, count: usize, recent_notes: usize) -> (App, PathBuf) {
    let dir = std::env::temp_dir().join(format!("nota-smoke-{}-{tag}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    for i in 0..count {
        let day = i + 1;
        let date = format!("2026-01-{day:02}");
        let post = dir.join(format!("posts/2026/01/{day:02}"));
        std::fs::create_dir_all(&post).expect("作れる");
        let body = format!(
            "# {date}\n\n<!-- acta:comment\nid: id-{day}\ncreated: {date} 09:00\ncreated_ms: {day}\ntags: ToDo\n-->\n# ToDo: {date}\n- グループ\n  - [ ] タスク{day}\n<!-- /acta:comment -->\n\n"
        );
        std::fs::write(post.join(format!("{date}.md")), body).expect("書ける");
    }
    let mut app = App::new(Config {
        data_dir: dir.clone(),
        source: "test".into(),
        recent_notes,
        project_done_limit: 5,
    })
    .expect("起動できる");
    app.dismiss_splash();
    (app, dir)
}

/// 一覧は既定で直近だけを出し、`a` で全件に切り替わる。
#[test]
fn note_list_starts_narrowed_and_expands() {
    let (mut app, dir) = seeded_app("recent", 5, 2);
    assert_eq!(app.notes.len(), 5);
    assert_eq!(app.visible_notes(), 2, "直近 2 件に絞られていない");

    // 絞り込み中は範囲外へカーソルが出ない。
    for _ in 0..10 {
        app.update(Msg::Move(Move::Down));
    }
    assert_eq!(app.note_sel, 1);

    app.update(Msg::ToggleAllNotes);
    assert_eq!(app.visible_notes(), 5, "全件に切り替わっていない");
    app.update(Msg::Move(Move::Bottom));
    assert_eq!(app.note_sel, 4);

    // 全件で末尾を選んでから絞り込むと、選択が範囲内に戻る。
    app.update(Msg::ToggleAllNotes);
    assert_eq!(app.visible_notes(), 2);
    assert_eq!(app.note_sel, 1, "選択が範囲外に残っている");

    let _ = std::fs::remove_dir_all(&dir);
}

/// 状態の違う ToDo を持つ一時データ。
fn seeded_todos(tag: &str) -> (App, PathBuf) {
    let dir = std::env::temp_dir().join(format!("nota-todos-{}-{tag}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    // 日付が新しいほうが先に出る。中身は状態を散らしておく。
    let days = [
        (1, "Beta", "[x] 完了した作業"),
        (2, "Alpha", "[-] 進行中の作業"),
        (3, "Beta", "[ ] 未着手の作業"),
    ];
    for (day, group, task) in days {
        let post = dir.join(format!("posts/2026/01/{day:02}"));
        std::fs::create_dir_all(&post).expect("作れる");
        let date = format!("2026-01-{day:02}");
        let body = format!(
            "# {date}\n\n<!-- acta:comment\nid: id-{day}\ncreated: {date} 09:00\ncreated_ms: {day}\ntags: ToDo\n-->\n# ToDo: {date}\n- {group}\n  - {task}\n<!-- /acta:comment -->\n\n"
        );
        std::fs::write(post.join(format!("{date}.md")), body).expect("書ける");
    }
    let mut app = App::new(Config {
        data_dir: dir.clone(),
        source: "test".into(),
        recent_notes: 30,
        project_done_limit: 5,
    })
    .expect("起動できる");
    app.dismiss_splash();
    app.update(Msg::SwitchView(View::Todo));
    (app, dir)
}

fn todo_titles(app: &App) -> Vec<String> {
    app.visible_todos()
        .iter()
        .map(|(_, item)| item.title.clone())
        .collect()
}

/// 既定は日付の新しい順。
#[test]
fn todos_are_sorted_by_date_first() {
    let (app, dir) = seeded_todos("date");
    assert_eq!(app.todo_sort, TodoSort::Date);
    assert_eq!(
        todo_titles(&app),
        vec!["未着手の作業", "進行中の作業", "完了した作業"]
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// f で未完だけに絞れる。
#[test]
fn f_shows_only_open_todos() {
    let (mut app, dir) = seeded_todos("open");
    app.update(Msg::ToggleTodoFilter);
    assert!(app.todo_open_only);
    assert_eq!(todo_titles(&app), vec!["未着手の作業", "進行中の作業"]);

    // 見出しにも出す。
    let out = squash(&render(&mut app, 100, 20));
    assert!(out.contains("未完のみ"), "表示が出ていない: {out}");

    // もう一度押すと戻る。
    app.update(Msg::ToggleTodoFilter);
    assert_eq!(todo_titles(&app).len(), 3);

    let _ = std::fs::remove_dir_all(&dir);
}

/// s で並び順が巡回する。
#[test]
fn s_cycles_the_sort_order() {
    let (mut app, dir) = seeded_todos("sort");

    app.update(Msg::CycleTodoSort);
    assert_eq!(app.todo_sort, TodoSort::Status);
    assert_eq!(
        todo_titles(&app),
        vec!["未着手の作業", "進行中の作業", "完了した作業"]
    );

    app.update(Msg::CycleTodoSort);
    assert_eq!(app.todo_sort, TodoSort::Project);
    // プロジェクト名順。同じ名前の中では日付の新しい順が残る。
    assert_eq!(
        todo_titles(&app),
        vec!["進行中の作業", "未着手の作業", "完了した作業"]
    );

    app.update(Msg::CycleTodoSort);
    assert_eq!(app.todo_sort, TodoSort::Date);

    let out = squash(&render(&mut app, 100, 20));
    assert!(out.contains("日付順"), "並び順が出ていない: {out}");

    let _ = std::fs::remove_dir_all(&dir);
}

/// 並びを変えても、選んでいた行を追いかける。
#[test]
fn changing_the_order_keeps_the_selection() {
    let (mut app, dir) = seeded_todos("keep");
    // 3 番目（完了した作業）を選ぶ。
    app.update(Msg::Move(Move::Bottom));
    assert_eq!(app.visible_todos()[app.todo_sel].1.title, "完了した作業");

    app.update(Msg::CycleTodoSort);
    assert_eq!(
        app.visible_todos()[app.todo_sel].1.title,
        "完了した作業",
        "別の行に移っている"
    );

    // 絞り込みで消える行を選んでいたら、先頭に戻す。
    app.update(Msg::ToggleTodoFilter);
    assert_eq!(app.visible_todos().len(), 2);
    assert!(app.todo_sel < 2, "範囲外を指している");

    let _ = std::fs::remove_dir_all(&dir);
}

/// 絞り込み中でも Space は正しい行に効く。
#[test]
fn space_hits_the_right_row_while_filtered() {
    let (mut app, dir) = seeded_todos("filtered");
    app.update(Msg::ToggleTodoFilter);
    // 未完のうち 2 番目（進行中の作業）。
    app.update(Msg::Move(Move::Down));
    assert_eq!(app.visible_todos()[app.todo_sel].1.title, "進行中の作業");

    app.update(Msg::CycleTodo);

    // 進行中 → 完了になり、絞り込みから外れる。
    let text = note_text(&dir, "2026-01-02");
    assert!(
        text.contains("- [x] 進行中の作業"),
        "別の行を変えている: {text}"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

/// ToDo は絞り込みに関係なく全期間から集める。
#[test]
fn todos_are_collected_from_all_notes() {
    let (app, dir) = seeded_app("todos", 5, 2);
    assert_eq!(app.todos.len(), 5, "絞り込みが ToDo にも効いてしまっている");
    let _ = std::fs::remove_dir_all(&dir);
}

/// 絞り込みの外にあるノートを検索から開くと、自動で全件表示になる。
#[test]
fn jumping_outside_the_window_expands_the_list() {
    let (mut app, dir) = seeded_app("jump", 5, 2);
    app.update(Msg::SearchStart);
    for c in "タスク1".chars() {
        app.update(Msg::SearchInput(c));
    }
    assert_eq!(app.hits.len(), 1);
    app.update(Msg::SearchCommit);
    assert!(app.show_all_notes, "全件表示に切り替わっていない");
    assert_eq!(app.note_sel, 4, "最古のノートが選ばれていない");

    let _ = std::fs::remove_dir_all(&dir);
}

/// recent_notes = 0 は「絞らない」の意味。
#[test]
fn zero_recent_notes_shows_everything() {
    let (app, dir) = seeded_app("zero", 4, 0);
    assert_eq!(app.visible_notes(), 4);
    let _ = std::fs::remove_dir_all(&dir);
}

fn note_text(dir: &Path, date: &str) -> String {
    let (year, rest) = date.split_at(4);
    let path = dir.join(format!(
        "posts/{year}/{}/{}/{date}.md",
        &rest[1..3],
        &rest[4..6]
    ));
    std::fs::read_to_string(path).expect("読める")
}

/// e で開いたエディタの結果が、本文だけを差し替えてファイルに入る。
#[test]
fn editing_an_entry_replaces_only_the_body() {
    let (mut app, dir) = seeded_app("edit", 2, 30);
    app.update(Msg::EditEntry);
    let request = app.take_edit_request().expect("エディタ要求が出る");
    assert_eq!(
        request.target,
        EditTarget::EntryBody {
            note_idx: 0,
            entry_idx: 0
        }
    );
    // 渡されるのは 1 行目のタグと本文だけ。メタ行やマーカーは含まない。
    assert!(request.initial.starts_with("tags: ToDo\n\n"));
    assert!(request.initial.contains("- [ ] タスク2"));
    assert!(!request.initial.contains("acta:comment"));
    assert!(!request.initial.contains("created_ms"));

    app.apply_edit(
        request.target,
        Some("tags: ToDo\n\n書き換えた本文\n".to_string()),
    );
    let text = note_text(&dir, "2026-01-02");
    assert!(text.contains("-->\n書き換えた本文\n<!-- /acta:comment -->"));
    assert!(text.contains("id: id-2"), "メタ行が失われている");
    assert!(text.contains("created_ms: 2"));
    assert!(text.ends_with("\n\n"), "末尾が変わっている");

    let _ = std::fs::remove_dir_all(&dir);
}

/// 変更なしで閉じたときは何も書かない。
#[test]
fn closing_the_editor_unchanged_writes_nothing() {
    let (mut app, dir) = seeded_app("noedit", 1, 30);
    let before = note_text(&dir, "2026-01-01");
    app.update(Msg::EditEntry);
    let request = app.take_edit_request().expect("要求が出る");
    app.apply_edit(request.target, None);
    assert_eq!(note_text(&dir, "2026-01-01"), before);
    let _ = std::fs::remove_dir_all(&dir);
}

/// 空にして閉じても消さない。誤操作でエントリが飛ぶのを防ぐ。
#[test]
fn emptying_the_body_is_rejected() {
    let (mut app, dir) = seeded_app("empty", 1, 30);
    let before = note_text(&dir, "2026-01-01");
    app.update(Msg::EditEntry);
    let request = app.take_edit_request().expect("要求が出る");
    app.apply_edit(request.target, Some("tags: ToDo\n\n   \n".to_string()));
    assert_eq!(note_text(&dir, "2026-01-01"), before);
    assert!(app.status.is_some());
    let _ = std::fs::remove_dir_all(&dir);
}

/// o で本文を書くと、タグを聞かれてから今日のノートに追記される。
#[test]
fn creating_an_entry_appends_to_today() {
    let (mut app, dir) = seeded_app("new", 1, 30);
    app.update(Msg::NewEntry);
    let request = app.take_edit_request().expect("要求が出る");
    assert_eq!(request.target, EditTarget::NewEntry);
    // タグ欄だけが入った状態で開く。
    assert_eq!(request.initial, "tags: \n\n");

    app.apply_edit(
        request.target,
        Some("tags: Memo, 検証\n\n新しく書いた本文\n".to_string()),
    );
    assert_eq!(app.mode, Mode::Normal);

    let today = chrono::Local::now().format("%Y-%m-%d").to_string();
    let text = note_text(&dir, &today);
    assert!(
        text.starts_with(&format!("# {today}\n\n")),
        "見出しがない: {text}"
    );
    assert!(text.contains("新しく書いた本文"));
    assert!(text.contains("tags: Memo, 検証"));
    assert!(text.contains("<!-- /acta:comment -->\n\n"));

    // 読み直しても壊れていない。
    let note = app.notes.iter().find(|n| n.date == today).expect("ある");
    assert_eq!(note.entries.len(), 1);
    assert_eq!(note.to_text(), text);

    let _ = std::fs::remove_dir_all(&dir);
}

/// 本文が空なら作らない。
#[test]
fn creating_with_an_empty_body_is_skipped() {
    let (mut app, dir) = seeded_app("newempty", 1, 30);
    app.update(Msg::NewEntry);
    let request = app.take_edit_request().expect("要求が出る");
    app.apply_edit(request.target, Some("tags: x\n\n  \n".to_string()));
    assert_eq!(app.notes.len(), 1, "ノートが増えている");
    let _ = std::fs::remove_dir_all(&dir);
}

/// D は確認を挟む。n なら消さない。
#[test]
fn deleting_asks_first() {
    let (mut app, dir) = seeded_app("delete", 2, 30);
    let before = note_text(&dir, "2026-01-02");
    app.update(Msg::DeleteEntry);
    assert_eq!(app.mode, Mode::Confirm);
    assert!(matches!(app.confirm, Some(Confirm::DeleteEntry { .. })));

    app.update(Msg::ConfirmNo);
    assert_eq!(app.mode, Mode::Normal);
    assert_eq!(note_text(&dir, "2026-01-02"), before);

    // y なら消える。
    app.update(Msg::DeleteEntry);
    app.update(Msg::ConfirmYes);
    let after = note_text(&dir, "2026-01-02");
    assert!(
        !after.contains("acta:comment"),
        "エントリが残っている: {after}"
    );
    assert!(
        after.starts_with("# 2026-01-02"),
        "見出しが消えている: {after}"
    );
    assert_eq!(app.todos.len(), 1, "ToDo 一覧が更新されていない");

    let _ = std::fs::remove_dir_all(&dir);
}

/// e でタグも編集できる。1 行目を書き換えるとメタ行に入る。
#[test]
fn editing_tags_from_the_first_line() {
    let (mut app, dir) = seeded_app("tags", 1, 30);
    app.update(Msg::EditEntry);
    let request = app.take_edit_request().expect("要求が出る");
    app.apply_edit(
        request.target,
        Some("tags: ToDo, 追加したタグ\n\n本文\n".to_string()),
    );

    let text = note_text(&dir, "2026-01-01");
    assert!(
        text.contains("tags: ToDo, 追加したタグ\n"),
        "タグが入っていない: {text}"
    );
    assert!(text.contains("id: id-1"), "メタ行が壊れている");
    assert_eq!(app.notes[0].entries[0].tags, vec!["ToDo", "追加したタグ"]);

    let _ = std::fs::remove_dir_all(&dir);
}

/// タグ行を消して保存したらタグなしになる。
#[test]
fn removing_the_tag_line_clears_tags() {
    let (mut app, dir) = seeded_app("notag", 1, 30);
    app.update(Msg::EditEntry);
    let request = app.take_edit_request().expect("要求が出る");
    app.apply_edit(request.target, Some("本文だけ残す\n".to_string()));

    let text = note_text(&dir, "2026-01-01");
    assert!(text.contains("tags: \n"), "タグが空になっていない: {text}");
    assert!(app.notes[0].entries[0].tags.is_empty());
    let _ = std::fs::remove_dir_all(&dir);
}

/// 検索の入力中でも Tab で Menu を回せる。
#[test]
fn tab_cycles_menu_while_searching() {
    let (mut app, dir) = seeded_app("tab", 1, 30);
    app.update(Msg::SearchStart);
    assert_eq!(app.view, View::Search);
    assert_eq!(app.mode, Mode::Search);

    // 検索の次はノートに回り、入力モードも抜ける。
    app.update(Msg::NextView);
    assert_eq!(app.view, View::Notes);
    assert_eq!(app.mode, Mode::Normal, "入力モードが残っている");

    // 一周して戻る。
    for _ in 0..3 {
        app.update(Msg::NextView);
    }
    assert_eq!(app.view, View::Search);
    assert_eq!(app.mode, Mode::Search, "検索に戻ったら入力できる");

    // 逆回しも同じ。
    app.update(Msg::PrevView);
    assert_eq!(app.view, View::Projects);
    assert_eq!(app.mode, Mode::Normal);

    let _ = std::fs::remove_dir_all(&dir);
}

/// 検索は Esc 一発で抜けて、開く前のビューに戻る。
#[test]
fn escape_leaves_search_for_the_previous_view() {
    let (mut app, dir) = seeded_app("escape", 2, 30);

    // ノートビューから開いたらノートビューに戻る。
    app.update(Msg::SearchStart);
    assert_eq!(app.view, View::Search);
    assert_eq!(app.mode, Mode::Search);
    app.update(Msg::SearchCancel);
    assert_eq!(app.view, View::Notes, "元のビューに戻っていない");
    assert_eq!(app.mode, Mode::Normal);

    // ToDo ビューから開いたら ToDo ビューに戻る。
    app.update(Msg::SwitchView(View::Todo));
    app.update(Msg::SearchStart);
    app.update(Msg::SearchCancel);
    assert_eq!(app.view, View::Todo, "元のビューに戻っていない");

    // 4 で直接開いた場合も同じ。
    app.update(Msg::SwitchView(View::Projects));
    app.update(Msg::SwitchView(View::Search));
    app.update(Msg::SearchCancel);
    assert_eq!(app.view, View::Projects);

    let _ = std::fs::remove_dir_all(&dir);
}

/// 確認待ちは画面下部に出る。
#[test]
fn footer_shows_the_confirmation() {
    let (mut app, dir) = seeded_app("footer", 1, 30);
    app.update(Msg::DeleteEntry);
    let out = squash(&render(&mut app, 100, 20));
    assert!(out.contains("削除しますか"), "確認が出ていない");
    let _ = std::fs::remove_dir_all(&dir);
}

/// プロジェクトを持つ一時データを作る。archived は archivedAtMs で表す。
fn seeded_projects(tag: &str) -> (App, PathBuf) {
    let dir = std::env::temp_dir().join(format!("nota-pj-{}-{tag}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);

    let make = |name: &str, slug: &str, archived: i64, tasks: &str| {
        let d = dir.join(format!("projects/{slug}"));
        std::fs::create_dir_all(&d).expect("作れる");
        let json = format!(
            r#"{{
  "id": "id-{slug}",
  "name": "{name}",
  "dirName": "{slug}",
  "createdAtMs": 1,
  "updatedAtMs": 2,
  "archivedAtMs": {archived},
  "issueUrl": "",
  "tasks": [{tasks}]
}}"#
        );
        std::fs::write(d.join("project.json"), json).expect("書ける");
    };

    // 未知の項目 sourceType / sourceState を混ぜて、書き戻しで消えないか見る。
    let tasks = r#"
    {"id":"t1","title":"進行中のタスク","status":"InProgress","createdAtMs":1,"updatedAtMs":1,"completedAtMs":0,"source":"github","sourceUrl":"u","repository":"r","sourceType":"issue","sourceState":"open"},
    {"id":"t2","title":"未着手のタスク","status":"Backlog","createdAtMs":1,"updatedAtMs":1,"completedAtMs":0,"source":"local","sourceUrl":"","repository":""},
    {"id":"t3","title":"完了1","status":"Done","createdAtMs":1,"updatedAtMs":1,"completedAtMs":9,"source":"local","sourceUrl":"","repository":""},
    {"id":"t4","title":"完了2","status":"Done","createdAtMs":1,"updatedAtMs":1,"completedAtMs":9,"source":"local","sourceUrl":"","repository":""},
    {"id":"t5","title":"完了3","status":"Done","createdAtMs":1,"updatedAtMs":1,"completedAtMs":9,"source":"local","sourceUrl":"","repository":""}
  "#;
    make("現役プロジェクト", "active", 0, tasks);
    make("終わったプロジェクト", "old", 1_700_000_000_000, "");

    let mut app = App::new(Config {
        data_dir: dir.clone(),
        source: "test".into(),
        recent_notes: 30,
        project_done_limit: 2,
    })
    .expect("起動できる");
    app.dismiss_splash();
    app.update(Msg::SwitchView(View::Projects));
    (app, dir)
}

fn project_json(dir: &Path, slug: &str) -> serde_json::Value {
    let text =
        std::fs::read_to_string(dir.join(format!("projects/{slug}/project.json"))).expect("読める");
    serde_json::from_str(&text).expect("JSON として読める")
}

/// t は進行中のタスクを並べた雛形を出す。
#[test]
fn t_builds_todays_todo_from_in_progress_tasks() {
    let (mut app, dir) = seeded_projects("todo");
    app.update(Msg::TodayTodo);
    let request = app.take_edit_request().expect("要求が出る");
    assert_eq!(request.target, EditTarget::NewEntry);

    let today = chrono::Local::now().date_naive();
    let heading = crate::model::todo_heading(today);
    assert!(
        request.initial.starts_with("tags: ToDo\n\n"),
        "ToDo タグが入っていない: {}",
        request.initial
    );
    assert!(
        request.initial.contains(&format!("# {heading}")),
        "見出しがない"
    );
    // 進行中のタスクだけが並ぶ。未着手と完了は入らない。
    assert!(request
        .initial
        .contains("- 現役プロジェクト\n  - [-] 進行中のタスク"));
    assert!(!request.initial.contains("未着手のタスク"));
    assert!(!request.initial.contains("完了1"));
    // アーカイブ済みのプロジェクトは対象外。
    assert!(!request.initial.contains("終わったプロジェクト"));

    // そのまま保存すると今日のノートに入る。
    app.apply_edit(request.target, Some(request.initial.clone()));
    let date = today.format("%Y-%m-%d").to_string();
    let text = note_text(&dir, &date);
    assert!(text.contains("tags: ToDo\n"));
    assert!(text.contains(&heading));

    // ToDo 一覧にも出る。
    assert_eq!(app.todos.len(), 1);
    assert_eq!(app.todos[0].1.group, "現役プロジェクト");
    assert_eq!(app.todos[0].1.title, "進行中のタスク");

    let _ = std::fs::remove_dir_all(&dir);
}

/// 今日の ToDo が既にあるなら、新しく作らずそれを開く。
#[test]
fn t_opens_the_existing_todo() {
    let (mut app, dir) = seeded_projects("todoexist");

    // 1 回目で作る。
    app.update(Msg::TodayTodo);
    let request = app.take_edit_request().expect("要求が出る");
    app.apply_edit(request.target, Some(request.initial.clone()));
    assert_eq!(app.todos.len(), 1);

    // 2 回目は既存のエントリを開く。
    app.update(Msg::TodayTodo);
    let request = app.take_edit_request().expect("要求が出る");
    assert!(
        matches!(request.target, EditTarget::EntryBody { .. }),
        "新規として開こうとしている: {:?}",
        request.target
    );
    assert!(request.initial.contains("進行中のタスク"));

    // 追記して保存しても、エントリは増えない。
    app.apply_edit(
        request.target,
        Some(format!(
            "{}\n  - [ ] 手で足したタスク\n",
            request.initial.trim_end()
        )),
    );
    let today = chrono::Local::now()
        .date_naive()
        .format("%Y-%m-%d")
        .to_string();
    let note = app.notes.iter().find(|n| n.date == today).expect("ある");
    assert_eq!(note.entries.len(), 1, "エントリが増えている");
    assert_eq!(app.todos.len(), 2);

    let _ = std::fs::remove_dir_all(&dir);
}

/// 進行中のタスクが無くても、見出しだけの雛形を出す。
#[test]
fn t_works_without_in_progress_tasks() {
    let (mut app, dir) = seeded_app("todoempty", 1, 30);
    app.update(Msg::TodayTodo);
    let request = app.take_edit_request().expect("要求が出る");
    let heading = crate::model::todo_heading(chrono::Local::now().date_naive());
    assert_eq!(request.initial, format!("tags: ToDo\n\n# {heading}"));
    let _ = std::fs::remove_dir_all(&dir);
}

/// プロジェクトビューの o で新しいプロジェクトを作れる。
#[test]
fn creating_a_project() {
    let (mut app, dir) = seeded_projects("create");
    app.update(Msg::NewEntry);
    let request = app.take_edit_request().expect("要求が出る");
    assert_eq!(request.target, EditTarget::NewProject);
    assert!(request.initial.is_empty(), "空から書き始める");

    app.apply_edit(request.target, Some("新しい取り組み Depot\n".to_string()));

    // 作ったものが選ばれた状態になる。
    let selected = app.selected_project().expect("ある");
    assert_eq!(selected.name, "新しい取り組み Depot");
    assert_eq!(selected.dir_name, "depot");
    assert!(selected.tasks.is_empty());
    assert!(dir.join("projects/depot/project.json").is_file());
    assert!(dir.join("projects/depot/knowledge.md").is_file());

    // 続けてタスクを足せる。
    app.update(Msg::EditEntry);
    let request = app.take_edit_request().expect("要求が出る");
    // タスクが無いときは行の形を出す。空白のままだとどう書くか分からない。
    assert_eq!(request.initial, "- [ ] ", "雛形が出ていない");
    app.apply_edit(request.target, Some("- [-] 最初の作業\n".to_string()));
    assert_eq!(app.selected_project().expect("ある").tasks.len(), 1);

    let _ = std::fs::remove_dir_all(&dir);
}

/// 新規プロジェクトで、チェックボックス無しに書いても反映されるか。
#[test]
fn plain_lines_become_tasks() {
    let (mut app, dir) = seeded_projects("plain");
    app.update(Msg::NewEntry);
    let request = app.take_edit_request().expect("要求が出る");
    app.apply_edit(request.target, Some("新規\n".to_string()));

    app.update(Msg::EditEntry);
    let request = app.take_edit_request().expect("要求が出る");
    // 箇条書きの記法を知らずに、名前だけ並べて保存した場合。
    app.apply_edit(request.target, Some("最初の作業\n次の作業\n".to_string()));

    let tasks = &app.selected_project().expect("ある").tasks;
    assert_eq!(tasks.len(), 2, "タスクにならなかった");
    assert_eq!(tasks[0].title, "最初の作業");

    let _ = std::fs::remove_dir_all(&dir);
}

/// 雛形のまま何も書かずに閉じたときは、そう伝えて何もしない。
#[test]
fn saving_the_bare_template_reports_nothing_read() {
    let (mut app, dir) = seeded_projects("template");
    app.update(Msg::NewEntry);
    let request = app.take_edit_request().expect("要求が出る");
    app.apply_edit(request.target, Some("新規\n".to_string()));

    app.update(Msg::EditEntry);
    let request = app.take_edit_request().expect("要求が出る");
    app.apply_edit(request.target, Some("- [ ] \n".to_string()));

    assert!(app.selected_project().expect("ある").tasks.is_empty());
    let status = app.status.clone().expect("知らせがある");
    assert!(
        status.contains("読み取れる行がありません"),
        "黙って終わっている: {status}"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

/// 既存プロジェクトでも、記法を混ぜて書ける。
#[test]
fn mixed_notation_is_accepted() {
    let (mut app, dir) = seeded_projects("mixed");
    app.update(Msg::EditEntry);
    let request = app.take_edit_request().expect("要求が出る");
    app.apply_edit(
        request.target,
        Some("- [-] 進行中のタスク\n記法なしで足す\n- 印だけ付けて足す\n".to_string()),
    );

    let tasks = &app.selected_project().expect("ある").tasks;
    let titles: Vec<&str> = tasks.iter().map(|t| t.title.as_str()).collect();
    assert_eq!(
        titles,
        vec!["進行中のタスク", "記法なしで足す", "印だけ付けて足す"]
    );
    assert_eq!(tasks[1].status, TaskStatus::Backlog);

    let _ = std::fs::remove_dir_all(&dir);
}

/// 名前が空なら作らない。
#[test]
fn creating_a_project_needs_a_name() {
    let (mut app, dir) = seeded_projects("noname");
    let before = app.projects.len();
    app.update(Msg::NewEntry);
    let request = app.take_edit_request().expect("要求が出る");
    app.apply_edit(request.target, Some("\n   \n".to_string()));
    assert_eq!(app.projects.len(), before, "作られている");
    let _ = std::fs::remove_dir_all(&dir);
}

/// ノートビューの o は今までどおりエントリを作る。
#[test]
fn o_still_creates_an_entry_outside_the_project_view() {
    let (mut app, dir) = seeded_projects("oentry");
    app.update(Msg::SwitchView(View::Notes));
    app.update(Msg::NewEntry);
    let request = app.take_edit_request().expect("要求が出る");
    assert_eq!(request.target, EditTarget::NewEntry);
    let _ = std::fs::remove_dir_all(&dir);
}

/// J / K で並びを入れ替えられる。並びは保存されて次回も残る。
#[test]
fn reordering_projects() {
    let (mut app, dir) = seeded_projects("reorder2");
    app.update(Msg::ToggleArchived);
    let names = |app: &App| -> Vec<String> {
        app.visible_projects()
            .iter()
            .map(|i| app.projects[*i].name.clone())
            .collect()
    };
    let before = names(&app);
    assert_eq!(before.len(), 2);

    // 先頭を下へ。
    app.update(Msg::MoveProject(1));
    let after = names(&app);
    assert_eq!(
        after,
        vec![before[1].clone(), before[0].clone()],
        "入れ替わっていない"
    );
    // カーソルは動かしたものを追う。
    assert_eq!(app.selected_project().expect("ある").name, before[0]);

    // 端では動かない。
    app.update(Msg::MoveProject(1));
    assert_eq!(names(&app), after);

    // 上に戻す。
    app.update(Msg::MoveProject(-1));
    assert_eq!(names(&app), before);

    // 保存されているので、開き直しても同じ並び。
    app.update(Msg::MoveProject(1));
    let expected = names(&app);
    let mut again = App::new(Config {
        data_dir: dir.clone(),
        source: "test".into(),
        recent_notes: 30,
        project_done_limit: 2,
    })
    .expect("起動できる");
    again.dismiss_splash();
    again.update(Msg::ToggleArchived);
    assert_eq!(names(&again), expected, "並びが残っていない");

    let _ = std::fs::remove_dir_all(&dir);
}

/// 新しく作ったプロジェクトも並びの中に入る。
#[test]
fn a_new_project_joins_the_order() {
    let (mut app, dir) = seeded_projects("neworder");
    app.update(Msg::MoveProject(1));
    app.update(Msg::NewEntry);
    let request = app.take_edit_request().expect("要求が出る");
    app.apply_edit(request.target, Some("later\n".to_string()));
    // 作った直後に選ばれていて、並びも壊れていない。
    assert_eq!(app.selected_project().expect("ある").name, "later");
    assert_eq!(app.visible_projects().len(), 2);
    let _ = std::fs::remove_dir_all(&dir);
}

/// 保存すると updatedAtMs が変わり一覧の並びが変わる。
/// 選択とタスクの対応が添字ごとずれないことを確かめる。
#[test]
fn saving_keeps_the_selected_project() {
    let (mut app, dir) = seeded_projects("reorder");
    app.update(Msg::ToggleArchived);
    // 更新が古い順に並ぶので、2 件目を選んでから保存する。
    app.update(Msg::Move(Move::Down));
    let picked = app.selected_project().expect("ある").name.clone();

    app.update(Msg::EditEntry);
    let request = app.take_edit_request().expect("要求が出る");
    app.apply_edit(
        request.target,
        Some(format!("{}- [-] 足した作業\n", request.initial)),
    );

    let after = app.selected_project().expect("ある");
    assert_eq!(after.name, picked, "選択が別のプロジェクトに移っている");
    assert!(
        after.tasks.iter().any(|t| t.title == "足した作業"),
        "別のプロジェクトに書かれている"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

/// t で今日の ToDo を用意してから、プロジェクト側の操作を反映させる。
fn with_todays_todo(tag: &str) -> (App, PathBuf) {
    let (mut app, dir) = seeded_projects(tag);
    app.update(Msg::TodayTodo);
    let request = app.take_edit_request().expect("要求が出る");
    app.apply_edit(request.target, Some(request.initial.clone()));
    app.update(Msg::SwitchView(View::Projects));
    (app, dir)
}

/// プロジェクトにタスクを足すと、今日の ToDo にも出る。
#[test]
fn adding_a_project_task_reaches_the_todo_view() {
    let (mut app, dir) = with_todays_todo("sync");
    // 雛形の時点では進行中の 1 件だけ。
    assert_eq!(app.todos.len(), 1);

    app.update(Msg::EditEntry);
    let request = app.take_edit_request().expect("要求が出る");
    app.apply_edit(
        request.target,
        Some(format!("{}- [-] 追加した作業\n", request.initial)),
    );

    // ToDo ビューに増えている。
    assert_eq!(app.todos.len(), 2, "ToDo に反映されていない");
    let titles: Vec<&str> = app.todos.iter().map(|(_, t)| t.title.as_str()).collect();
    assert!(titles.contains(&"追加した作業"));
    assert_eq!(app.todos[1].1.group, "現役プロジェクト");

    // ファイルにも入っている。
    let today = chrono::Local::now()
        .date_naive()
        .format("%Y-%m-%d")
        .to_string();
    let text = note_text(&dir, &today);
    assert!(
        text.contains("  - [-] 追加した作業\n"),
        "書けていない: {text}"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

/// 未着手のタスクを足しただけでは ToDo に出さない。着手していない予定で埋めない。
#[test]
fn backlog_tasks_do_not_reach_the_todo_view() {
    let (mut app, dir) = with_todays_todo("backlog");
    app.update(Msg::EditEntry);
    let request = app.take_edit_request().expect("要求が出る");
    app.apply_edit(
        request.target,
        Some(format!("{}- [ ] まだ着手しない\n", request.initial)),
    );
    assert_eq!(app.todos.len(), 1, "未着手まで入っている");
    let _ = std::fs::remove_dir_all(&dir);
}

/// プロジェクト側で状態を変えると、今日の ToDo のチェック欄も追従する。
#[test]
fn changing_a_task_status_updates_the_todo_view() {
    let (mut app, dir) = with_todays_todo("status");
    assert_eq!(app.todos[0].1.status, TaskStatus::InProgress);

    app.update(Msg::ToggleFocus);
    app.update(Msg::CycleTodo);

    // InProgress → Done が ToDo 側にも入る。
    app.update(Msg::SwitchView(View::Todo));
    assert_eq!(app.todos.len(), 1, "行が増えている");
    assert_eq!(app.todos[0].1.status, TaskStatus::Done, "追従していない");

    let today = chrono::Local::now()
        .date_naive()
        .format("%Y-%m-%d")
        .to_string();
    assert!(note_text(&dir, &today).contains("  - [x] 進行中のタスク\n"));

    let _ = std::fs::remove_dir_all(&dir);
}

/// ToDo でチェックを入れると、プロジェクトのタスクにも入る。
#[test]
fn checking_a_todo_updates_the_project() {
    let (mut app, dir) = with_todays_todo("back");
    app.update(Msg::SwitchView(View::Todo));
    assert_eq!(app.visible_todos()[0].1.title, "進行中のタスク");

    app.update(Msg::CycleTodo);

    // ToDo 側は完了になる。
    let today = chrono::Local::now()
        .date_naive()
        .format("%Y-%m-%d")
        .to_string();
    assert!(note_text(&dir, &today).contains("- [x] 進行中のタスク"));

    // プロジェクト側にも入る。
    let json = project_json(&dir, "active");
    let task = json["tasks"]
        .as_array()
        .expect("配列")
        .iter()
        .find(|t| t["title"] == "進行中のタスク")
        .expect("ある");
    assert_eq!(task["status"], "Done", "プロジェクトに反映されていない");
    assert!(task["completedAtMs"].as_i64().expect("数値") > 0);
    assert_eq!(task["id"], "t1", "別のタスクとして作り直されている");
    // 解釈しない項目も残る。
    assert_eq!(task["sourceType"], "issue");

    let _ = std::fs::remove_dir_all(&dir);
}

/// エディタでチェックを書き換えた分もプロジェクトに入る。
#[test]
fn editing_checkboxes_in_the_editor_reaches_the_project() {
    let (mut app, dir) = with_todays_todo("manual");
    app.update(Msg::SwitchView(View::Todo));

    // ToDo をエディタで開き、チェック欄を直接書き換える。
    app.update(Msg::EditEntry);
    let request = app.take_edit_request().expect("要求が出る");
    assert!(request.initial.contains("- [-] 進行中のタスク"));
    app.apply_edit(
        request.target,
        Some(
            request
                .initial
                .replace("- [-] 進行中のタスク", "- [x] 進行中のタスク"),
        ),
    );

    let json = project_json(&dir, "active");
    let task = json["tasks"]
        .as_array()
        .expect("配列")
        .iter()
        .find(|t| t["title"] == "進行中のタスク")
        .expect("ある");
    assert_eq!(task["status"], "Done", "プロジェクトに反映されていない");
    assert_eq!(task["id"], "t1", "作り直されている");

    let _ = std::fs::remove_dir_all(&dir);
}

/// 複数行をまとめて書き換えても、全部反映する。
#[test]
fn editing_many_checkboxes_at_once() {
    let (mut app, dir) = seeded_projects("many");

    // 2 件を進行中にしてから今日の ToDo を作る。
    app.update(Msg::EditEntry);
    let request = app.take_edit_request().expect("要求が出る");
    app.apply_edit(
        request.target,
        Some("- [-] 進行中のタスク\n- [-] 未着手のタスク\n".to_string()),
    );
    app.update(Msg::TodayTodo);
    let request = app.take_edit_request().expect("要求が出る");
    app.apply_edit(request.target, Some(request.initial.clone()));

    // ToDo 側で 2 行とも完了にする。
    app.update(Msg::SwitchView(View::Todo));
    app.update(Msg::EditEntry);
    let request = app.take_edit_request().expect("要求が出る");
    app.apply_edit(
        request.target,
        Some(request.initial.replace("- [-]", "- [x]")),
    );

    let json = project_json(&dir, "active");
    for title in ["進行中のタスク", "未着手のタスク"] {
        let task = json["tasks"]
            .as_array()
            .expect("配列")
            .iter()
            .find(|t| t["title"] == title)
            .expect("ある");
        assert_eq!(task["status"], "Done", "{title} が反映されていない");
    }

    let _ = std::fs::remove_dir_all(&dir);
}

/// 左ペインにいる間の Space は、一番上を変えずにタスク側へ移るだけ。
#[test]
fn space_moves_focus_before_changing_a_task() {
    let (mut app, dir) = seeded_projects("focus");
    assert_eq!(app.focus, Focus::List);
    let before = project_json(&dir, "active");

    app.update(Msg::CycleTodo);
    assert_eq!(app.focus, Focus::Detail, "タスク側へ移っていない");
    assert_eq!(
        project_json(&dir, "active"),
        before,
        "一番上を勝手に変えている"
    );

    // 2 件目を選んでから押すと、その行が変わる。
    app.update(Msg::Move(Move::Down));
    let target = app.visible_tasks()[app.task_sel].title.clone();
    assert_eq!(target, "未着手のタスク");
    app.update(Msg::CycleTodo);

    let json = project_json(&dir, "active");
    let tasks = json["tasks"].as_array().expect("配列");
    let changed = tasks.iter().find(|t| t["title"] == target).expect("ある");
    assert_eq!(changed["status"], "InProgress");
    // 一番上は元のまま。
    let top = tasks
        .iter()
        .find(|t| t["title"] == "進行中のタスク")
        .expect("ある");
    assert_eq!(top["status"], "InProgress", "別の行が変わっている");

    let _ = std::fs::remove_dir_all(&dir);
}

/// どのタスクが対象かは、フォーカスが無くても画面から分かる。
#[test]
fn the_task_cursor_is_always_visible() {
    let (mut app, dir) = seeded_projects("cursor");
    let out = render(&mut app, 100, 20);
    assert!(out.contains("▌"), "選択行の印が出ていない");
    assert!(squash(&out).contains("lで選ぶ"), "操作の案内が出ていない");

    app.update(Msg::ToggleFocus);
    let out = squash(&render(&mut app, 100, 20));
    assert!(out.contains("Spaceで状態を進める"), "案内が変わっていない");

    let _ = std::fs::remove_dir_all(&dir);
}

/// 一周させても ToDo とプロジェクトが食い違わない。
#[test]
fn cycling_keeps_both_sides_in_step() {
    let (mut app, dir) = with_todays_todo("roundtrip");
    app.update(Msg::SwitchView(View::Todo));

    for expected in ["Done", "Backlog", "InProgress"] {
        app.update(Msg::CycleTodo);
        let json = project_json(&dir, "active");
        let task = json["tasks"]
            .as_array()
            .expect("配列")
            .iter()
            .find(|t| t["title"] == "進行中のタスク")
            .expect("ある");
        assert_eq!(task["status"], expected, "ずれている");
        // ToDo 側の行も同じ状態。
        assert_eq!(
            app.visible_todos()[0].1.status.name(),
            expected,
            "ToDo 側とプロジェクト側が食い違っている"
        );
    }

    // 行は増えていない。反映が往復して重複しない。
    assert_eq!(app.todos.len(), 1);
    assert_eq!(
        project_json(&dir, "active")["tasks"]
            .as_array()
            .expect("配列")
            .len(),
        5
    );

    let _ = std::fs::remove_dir_all(&dir);
}

/// プロジェクトに無い行は ToDo だけの予定として扱う。
#[test]
fn a_row_without_a_project_stays_local() {
    let (mut app, dir) = with_todays_todo("localrow");

    // ToDo に手で 1 行足す。
    app.update(Msg::SwitchView(View::Todo));
    app.update(Msg::EditEntry);
    let request = app.take_edit_request().expect("要求が出る");
    app.apply_edit(
        request.target,
        Some(format!(
            "{}\n  - [ ] 手で足した予定\n",
            request.initial.trim_end()
        )),
    );

    let at = app
        .visible_todos()
        .iter()
        .position(|(_, t)| t.title == "手で足した予定")
        .expect("ある");
    for _ in 0..at {
        app.update(Msg::Move(Move::Down));
    }
    app.update(Msg::CycleTodo);

    // プロジェクトのタスクは増えない。
    let json = project_json(&dir, "active");
    let titles: Vec<&str> = json["tasks"]
        .as_array()
        .expect("配列")
        .iter()
        .map(|t| t["title"].as_str().expect("文字列"))
        .collect();
    assert!(
        !titles.contains(&"手で足した予定"),
        "プロジェクトに混ざっている"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

/// 今日の ToDo がまだ無いときは、勝手にノートを作らない。
#[test]
fn syncing_does_nothing_without_a_todo() {
    let (mut app, dir) = seeded_projects("nosync");
    let before = app.notes.len();

    app.update(Msg::ToggleFocus);
    app.update(Msg::CycleTodo);

    assert_eq!(app.notes.len(), before, "ノートが増えている");
    assert!(app.todos.is_empty());
    // プロジェクト側の変更自体は保存されている。
    let json = project_json(&dir, "active");
    let task = json["tasks"]
        .as_array()
        .expect("配列")
        .iter()
        .find(|t| t["title"] == "進行中のタスク")
        .expect("ある");
    assert_eq!(task["status"], "Done");

    let _ = std::fs::remove_dir_all(&dir);
}

/// ToDo 側で手を入れた行は、反映で消さない。
#[test]
fn syncing_keeps_rows_added_by_hand() {
    let (mut app, dir) = with_todays_todo("byhand");

    // ToDo に手で 1 行足す。
    app.update(Msg::SwitchView(View::Todo));
    app.update(Msg::EditEntry);
    let request = app.take_edit_request().expect("要求が出る");
    app.apply_edit(
        request.target,
        Some(format!(
            "{}\n  - [ ] 手で足した予定\n",
            request.initial.trim_end()
        )),
    );
    assert_eq!(app.todos.len(), 2);

    // プロジェクト側をいじっても残る。
    app.update(Msg::SwitchView(View::Projects));
    app.update(Msg::ToggleFocus);
    app.update(Msg::CycleTodo);

    let titles: Vec<&str> = app.todos.iter().map(|(_, t)| t.title.as_str()).collect();
    assert!(titles.contains(&"手で足した予定"), "消えている: {titles:?}");

    let _ = std::fs::remove_dir_all(&dir);
}

/// アーカイブ済みは既定で出さず、A で出る。
#[test]
fn archived_projects_are_hidden_by_default() {
    let (mut app, dir) = seeded_projects("archived");
    assert_eq!(app.projects.len(), 2, "読み込み自体は全件");
    assert_eq!(app.visible_projects().len(), 1, "既定で絞られていない");
    assert_eq!(
        app.selected_project().expect("ある").name,
        "現役プロジェクト"
    );

    let out = squash(&render(&mut app, 100, 20));
    assert!(!out.contains("終わったプロジェクト"), "隠れていない");

    app.update(Msg::ToggleArchived);
    assert_eq!(app.visible_projects().len(), 2);
    let out = squash(&render(&mut app, 100, 20));
    assert!(out.contains("終わったプロジェクト"), "出ていない");

    // 末尾を選んでから隠すと、選択が範囲内に戻る。
    app.update(Msg::Move(Move::Bottom));
    assert_eq!(app.project_sel, 1);
    app.update(Msg::ToggleArchived);
    assert_eq!(app.project_sel, 0, "選択が範囲外に残っている");

    let _ = std::fs::remove_dir_all(&dir);
}

/// 完了タスクは既定で数を絞り、隠した件数を見出しに出す。
#[test]
fn done_tasks_are_capped() {
    let (mut app, dir) = seeded_projects("done");
    // 進行中 1 + 未着手 1 + 完了 2（上限）。
    assert_eq!(app.visible_tasks().len(), 4);
    assert_eq!(app.hidden_done(), 1);

    let out = squash(&render(&mut app, 100, 20));
    assert!(out.contains("完了他1件"), "隠した件数が出ていない: {out}");
    assert!(out.contains("完了1") && out.contains("完了2"));
    assert!(!out.contains("完了3"), "上限を超えて出ている");

    let _ = std::fs::remove_dir_all(&dir);
}

/// 上限 0 は「絞らない」の意味。
#[test]
fn zero_done_limit_shows_everything() {
    let (mut app, dir) = seeded_projects("nolimit");
    app.config.project_done_limit = 0;
    assert_eq!(app.visible_tasks().len(), 5);
    assert_eq!(app.hidden_done(), 0);
    let _ = std::fs::remove_dir_all(&dir);
}

/// Space でタスクの状態が進み、project.json に書かれる。未知の項目は残る。
#[test]
fn space_cycles_a_project_task() {
    let (mut app, dir) = seeded_projects("cycle");
    app.update(Msg::ToggleFocus);
    assert_eq!(app.focus, Focus::Detail);

    // 先頭は進行中のタスク。次は完了になる。
    app.update(Msg::CycleTodo);
    let json = project_json(&dir, "active");
    let task = json["tasks"]
        .as_array()
        .expect("配列")
        .iter()
        .find(|t| t["title"] == "進行中のタスク")
        .expect("ある");
    assert_eq!(task["status"], "Done");
    assert!(
        task["completedAtMs"].as_i64().expect("数値") > 0,
        "完了時刻が入っていない"
    );
    // nota が解釈しない項目も残る。
    assert_eq!(task["sourceType"], "issue");
    assert_eq!(task["sourceState"], "open");
    assert_eq!(task["id"], "t1", "id が入れ替わっている");

    // 並びが変わってもカーソルは同じタスクを追うので、押し続けると一周する。
    app.update(Msg::CycleTodo);
    app.update(Msg::CycleTodo);
    let json = project_json(&dir, "active");
    let task = json["tasks"]
        .as_array()
        .expect("配列")
        .iter()
        .find(|t| t["title"] == "進行中のタスク")
        .expect("ある");
    assert_eq!(
        task["status"], "InProgress",
        "カーソルが別のタスクに移っている"
    );
    assert_eq!(task["completedAtMs"], 0, "完了時刻が残っている");

    let _ = std::fs::remove_dir_all(&dir);
}

/// e でタスク一覧をチェックリストとして編集できる。
#[test]
fn editing_project_tasks_as_a_checklist() {
    let (mut app, dir) = seeded_projects("edit");
    app.update(Msg::EditEntry);
    let request = app.take_edit_request().expect("要求が出る");
    // 進行中が上に来る。
    assert!(request.initial.starts_with(
        "- [-] 進行中のタスク
"
    ));
    assert!(request.initial.contains(
        "- [ ] 未着手のタスク
"
    ));
    assert!(request.initial.contains(
        "- [x] 完了1
"
    ));

    // 名前の変更、追加、削除をまとめて行う。
    app.apply_edit(
        request.target,
        Some(
            "- [-] 進行中のタスク
- [x] 未着手のタスク
- [ ] 追加したタスク
- [x] 完了1
"
            .to_string(),
        ),
    );

    let json = project_json(&dir, "active");
    let tasks = json["tasks"].as_array().expect("配列");
    assert_eq!(tasks.len(), 4, "件数が合わない");
    let titles: Vec<&str> = tasks
        .iter()
        .map(|t| t["title"].as_str().expect("文字列"))
        .collect();
    assert_eq!(
        titles,
        vec![
            "進行中のタスク",
            "未着手のタスク",
            "追加したタスク",
            "完了1"
        ]
    );
    // 状態の変更が入り、既存タスクは id を保つ。
    assert_eq!(tasks[1]["status"], "Done");
    assert_eq!(tasks[1]["id"], "t2");
    // 追加分には新しい id が振られる。
    assert_ne!(tasks[2]["id"], serde_json::Value::Null);
    assert_eq!(tasks[2]["status"], "Backlog");
    assert_eq!(tasks[2]["source"], "local");
    // 消した分は残らない。
    assert!(!titles.contains(&"完了2"));

    let _ = std::fs::remove_dir_all(&dir);
}

/// 全部消して保存されたときは変更しない。誤操作でタスクが飛ぶのを防ぐ。
#[test]
fn emptying_the_checklist_is_rejected() {
    let (mut app, dir) = seeded_projects("emptylist");
    app.update(Msg::EditEntry);
    let request = app.take_edit_request().expect("要求が出る");
    app.apply_edit(
        request.target,
        Some(
            "

"
            .to_string(),
        ),
    );

    let json = project_json(&dir, "active");
    assert_eq!(
        json["tasks"].as_array().expect("配列").len(),
        5,
        "消えている"
    );
    assert!(app.status.is_some());
    let _ = std::fs::remove_dir_all(&dir);
}

/// エントリの区切りはペイン幅まで伸びる。
#[test]
fn entry_separator_fills_the_pane() {
    let (mut app, dir) = seeded_app("sep", 1, 30);
    let screen = render(&mut app, 100, 20);
    let separator = screen
        .lines()
        .find(|l| l.contains("──"))
        .expect("区切りが出ていない");
    // 枠の直前まで伸びていること。
    assert!(
        separator.matches('─').count() > 40,
        "区切りが伸びていない: {separator}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// 低い端末ではヘルプが入りきらないので、送って読めるようにする。
#[test]
fn help_scrolls_on_a_short_terminal() {
    let mut app = empty_app();
    app.update(Msg::ToggleHelp);

    // 全部は入らない高さで開く。
    let first = squash(&render(&mut app, 110, 16));
    assert!(first.contains("キー操作"), "見出しが無い");
    assert!(first.contains("キー操作1/"), "位置が出ていない");
    let last = crate::keys::HELP.last().expect("項目がある").1;
    assert!(!first.contains(&squash(last)), "全部入ってしまっている");

    // 送ると後ろが見える。
    for _ in 0..30 {
        app.update(Msg::Move(Move::Down));
    }
    let scrolled = squash(&render(&mut app, 110, 16));
    assert!(
        scrolled.contains(&squash(last)),
        "送っても出てこない: {scrolled}"
    );

    // 高い端末なら位置は出ない。
    app.update(Msg::ToggleHelp);
    app.update(Msg::ToggleHelp);
    let tall = squash(&render(&mut app, 110, 40));
    assert!(
        !tall.contains("キー操作1/"),
        "収まっているのに位置が出ている"
    );
}

/// ヘルプは移動以外のキーで閉じる。
#[test]
fn help_closes_on_other_keys() {
    let mut app = empty_app();
    app.update(Msg::ToggleHelp);
    assert_eq!(app.mode, Mode::Help);
    // 送っても閉じない。
    app.update(Msg::Move(Move::Down));
    assert_eq!(app.mode, Mode::Help);
    // 閉じたらスクロール位置は戻る。
    app.update(Msg::ToggleHelp);
    assert_eq!(app.mode, Mode::Normal);
    assert_eq!(app.help_scroll, 0);
}

/// 起動直後はロゴが出て、キーを押すと通常の画面になる。
#[test]
fn splash_shows_on_start_and_dismisses() {
    let mut app = App::new(Config {
        data_dir: PathBuf::from("/nonexistent"),
        source: "test".into(),
        recent_notes: 30,
        project_done_limit: 5,
    })
    .expect("起動できる");

    assert!(app.splash_visible(), "起動直後にロゴが出ていない");
    let out = render(&mut app, 80, 24);
    assert!(out.contains("███"), "ロゴが描画されていない");
    // squash は空白を落とすので、比較する側も詰めておく。
    assert!(squash(&out).contains("Actaのデータをターミナルから"));

    app.dismiss_splash();
    assert!(!app.splash_visible());
    let out = squash(&render(&mut app, 80, 24));
    assert!(!out.contains("███"), "ロゴが残っている");
    assert!(out.contains("ノート"), "通常の画面になっていない");
}

/// 狭い端末でもロゴでパニックしない。
#[test]
fn splash_survives_a_tiny_terminal() {
    let mut app = App::new(Config {
        data_dir: PathBuf::from("/nonexistent"),
        source: "test".into(),
        recent_notes: 30,
        project_done_limit: 5,
    })
    .expect("起動できる");
    render(&mut app, 20, 4);
    render(&mut app, 8, 2);
}

/// ヘッダーも他のペインと同じ枠に入る。
#[test]
fn header_is_a_framed_menu() {
    let (mut app, dir) = seeded_app("menu", 1, 30);
    let screen = render(&mut app, 100, 20);
    let top = screen.lines().next().expect("1 行目がある");
    assert!(top.contains("Menu"), "Menu の見出しがない: {top}");
    assert!(top.starts_with('┌'), "枠になっていない: {top}");
    // タブは枠の中に入る。
    let second = screen.lines().nth(1).expect("2 行目がある");
    assert!(second.starts_with('│'), "枠の中にない: {second}");
    assert!(squash(second).contains("ノート"));
    let _ = std::fs::remove_dir_all(&dir);
}

/// 今の状態がタブ行の右端に出る。
#[test]
fn mode_badge_appears_in_the_tab_row() {
    let (mut app, dir) = seeded_app("badge", 1, 30);
    let out = squash(&render(&mut app, 100, 20));
    assert!(!out.contains("SEARCH"), "通常モードでバッジが出ている");

    app.update(Msg::SearchStart);
    let out = squash(&render(&mut app, 100, 20));
    assert!(out.contains("SEARCH"), "検索中のバッジが出ていない");

    app.update(Msg::SearchCancel);
    app.update(Msg::DeleteEntry);
    let out = squash(&render(&mut app, 100, 20));
    assert!(out.contains("CONFIRM"), "確認中のバッジが出ていない");

    let _ = std::fs::remove_dir_all(&dir);
}

/// データが空でも全ビューが描画できる。初回起動時に落ちないことの担保。
#[test]
fn renders_every_view_without_data() {
    let mut app = empty_app();
    for view in View::ALL {
        app.update(Msg::SwitchView(view));
        let out = render(&mut app, 100, 30);
        assert!(!out.trim().is_empty(), "{view:?} が空で描画された");
    }
}

/// 極端に狭い端末でもパニックしない。レイアウト計算の境界。
#[test]
fn survives_tiny_terminal() {
    let mut app = empty_app();
    for view in View::ALL {
        app.update(Msg::SwitchView(view));
        render(&mut app, 12, 5);
    }
}

#[test]
fn help_popup_shows_keys_and_config_source() {
    let mut app = empty_app();
    app.update(Msg::ToggleHelp);
    let out = squash(&render(&mut app, 110, 34));
    assert!(out.contains("キー操作"), "ヘルプの見出しが出ていない");
    // 説明が枠で切れていないこと。以前 $EDITOR の行が途切れていた。
    // 一覧の全行について、最後の 1 文字まで出ていることを見る。
    let squashed = squash(&render(&mut app, 110, 40));
    for (key, desc) in crate::keys::HELP {
        let tail: String = desc
            .chars()
            .rev()
            .take(6)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect();
        assert!(
            squashed.contains(&squash(&tail)),
            "{key} の説明が切れている: {desc}"
        );
    }
    assert!(out.contains("設定の出どころ"), "設定の出どころが出ていない");
    assert!(
        out.contains("/nonexistent"),
        "データディレクトリが出ていない"
    );
}

/// データが無い状態で移動キーを連打しても添字が壊れない。
#[test]
fn navigation_is_safe_when_lists_are_empty() {
    let mut app = empty_app();
    for view in View::ALL {
        app.update(Msg::SwitchView(view));
        for m in [
            Move::Down,
            Move::Bottom,
            Move::PageDown,
            Move::Up,
            Move::Top,
        ] {
            app.update(Msg::Move(m));
        }
    }
    app.update(Msg::CycleTodo);
    assert!(app.status.is_some(), "ToDo が無いことを知らせる");
    assert!(!app.should_quit);
}

#[test]
fn search_narrows_and_clears() {
    let mut app = empty_app();
    app.update(Msg::SearchStart);
    for c in "terraform".chars() {
        app.update(Msg::SearchInput(c));
    }
    assert_eq!(app.query, "terraform");
    app.update(Msg::SearchClear);
    assert!(app.query.is_empty());
    assert!(app.hits.is_empty());
}

/// 実データに対する健全性チェック。
/// 実行するには `NOTA_DATA_DIR` を指定して `cargo test -- --ignored` を使う。
#[test]
#[ignore = "実データが必要"]
fn reads_real_data() {
    let config = match Config::load(None) {
        Ok(config) => config,
        Err(err) => {
            // 設定が無い環境では検証しようがないので、理由を出して終わる。
            println!("実データが無いので省略します: {err}");
            println!("NOTA_DATA_DIR=/path/to/Acta cargo test -- --ignored で実行してください");
            return;
        }
    };
    println!(
        "data_dir: {} ({})",
        config.data_dir.display(),
        config.source
    );

    let mut app = App::new(config).expect("起動できる");
    app.dismiss_splash();
    assert!(!app.notes.is_empty(), "デイリーノートが 1 件も読めていない");

    // パースした全ノートで原文が復元できること。書き戻しの前提。
    for note in &app.notes {
        let text = std::fs::read_to_string(&note.path).expect("読める");
        let normalized = text.replace("\r\n", "\n").replace('\r', "\n");
        assert_eq!(
            note.to_text(),
            normalized,
            "原文を復元できない: {}",
            note.path.display()
        );
    }

    let entries: usize = app.notes.iter().map(|n| n.entries.len()).sum();
    println!(
        "notes={} entries={} todos={} projects={}",
        app.notes.len(),
        entries,
        app.todos.len(),
        app.projects.len()
    );
    assert!(entries > 0, "エントリが 1 件も読めていない");

    // 全ビューが実データで描画できる。
    for view in View::ALL {
        app.update(Msg::SwitchView(view));
        render(&mut app, 120, 40);
    }

    // 検索が動く。ヒットゼロでも落ちないことが要点。
    app.update(Msg::SearchStart);
    for c in "aws".chars() {
        app.update(Msg::SearchInput(c));
    }
    println!("hits for \"aws\": {}", app.hits.len());
    app.update(Msg::SearchCommit);
}
