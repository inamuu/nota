//! 描画と実データ読み込みの検証。
//!
//! TUI は目で見ないと分からない部分が多いので、少なくとも「落ちない」ことと
//! 「実データを解釈できる」ことは自動で確かめる。

#![cfg(test)]

use std::path::{Path, PathBuf};

use ratatui::backend::TestBackend;
use ratatui::Terminal;

use crate::app::{App, Confirm, Mode, Move, Msg, View};
use crate::config::Config;
use crate::editor::EditTarget;

fn empty_app() -> App {
    let mut app = App::new(Config {
        data_dir: PathBuf::from("/nonexistent"),
        source: "test".into(),
        recent_notes: 30,
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

/// 起動直後はロゴが出て、キーを押すと通常の画面になる。
#[test]
fn splash_shows_on_start_and_dismisses() {
    let mut app = App::new(Config {
        data_dir: PathBuf::from("/nonexistent"),
        source: "test".into(),
        recent_notes: 30,
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
    })
    .expect("起動できる");
    render(&mut app, 20, 4);
    render(&mut app, 8, 2);
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
    assert!(
        out.contains("エントリを作る（$EDITOR）"),
        "説明が切れている"
    );
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
