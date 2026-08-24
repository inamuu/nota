//! 描画と実データ読み込みの検証。
//!
//! TUI は目で見ないと分からない部分が多いので、少なくとも「落ちない」ことと
//! 「実データを解釈できる」ことは自動で確かめる。

#![cfg(test)]

use std::path::PathBuf;

use ratatui::backend::TestBackend;
use ratatui::Terminal;

use crate::app::{App, Move, Msg, View};
use crate::config::Config;

fn empty_app() -> App {
    App::new(Config {
        data_dir: PathBuf::from("/nonexistent"),
        source: "test".into(),
        recent_notes: 30,
    })
    .expect("データが無くても起動できる")
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
    let app = App::new(Config {
        data_dir: dir.clone(),
        source: "test".into(),
        recent_notes,
    })
    .expect("起動できる");
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
    let out = squash(&render(&mut app, 100, 30));
    assert!(out.contains("キー操作"), "ヘルプの見出しが出ていない");
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
    let config = Config::load(None).expect("データディレクトリを解決できる");
    println!(
        "data_dir: {} ({})",
        config.data_dir.display(),
        config.source
    );

    let mut app = App::new(config).expect("起動できる");
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
