//! キー入力を `Msg` に翻訳する。
//!
//! すべての操作をキーボードだけで完結させる。マウスは有効化しない。
//! モードごとに解釈を分けてあるので、挿入モードを足すときは分岐を 1 つ増やす。

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::app::{App, Focus, Mode, Move, Msg, View};

/// キーバインド一覧。ヘルプ画面がこの配列を描画するので、実装と説明がずれない。
pub const HELP: &[(&str, &str)] = &[
    ("j / k, ↓ / ↑", "選択を上下に移動"),
    ("Ctrl-d / Ctrl-u", "半画面ずつ移動"),
    ("g / G", "先頭 / 末尾へ"),
    ("1 / 2 / 3 / 4", "ノート / ToDo / プロジェクト / 検索"),
    ("Tab / Shift-Tab", "ビューを順に切り替え"),
    ("h / l", "一覧と本文のフォーカスを移動"),
    ("Enter", "ノートは本文へ、検索は該当箇所へジャンプ"),
    ("Space", "ToDo の状態を進める（未着手→進行中→完了）"),
    ("/", "全文検索"),
    ("a", "ノート一覧を直近だけ / 全件で切り替え"),
    ("r", "データを再読み込み"),
    ("?", "このヘルプ"),
    ("q, Ctrl-c", "終了"),
    ("Esc", "モードを抜ける / メッセージを消す"),
];

pub fn handle(app: &App, key: KeyEvent) -> Option<Msg> {
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);

    // Ctrl-c はどのモードでも終了。
    if ctrl && matches!(key.code, KeyCode::Char('c')) {
        return Some(Msg::Quit);
    }

    match app.mode {
        Mode::Help => match key.code {
            KeyCode::Char('q') => Some(Msg::Quit),
            _ => Some(Msg::ToggleHelp),
        },
        Mode::Search => search_mode(key, ctrl),
        Mode::Normal => normal_mode(app, key, ctrl),
    }
}

fn search_mode(key: KeyEvent, ctrl: bool) -> Option<Msg> {
    match key.code {
        KeyCode::Esc => Some(Msg::SearchCancel),
        KeyCode::Enter => Some(Msg::SearchCommit),
        KeyCode::Backspace => Some(Msg::SearchBackspace),
        KeyCode::Char('u') if ctrl => Some(Msg::SearchClear),
        KeyCode::Char('n') if ctrl => Some(Msg::Move(Move::Down)),
        KeyCode::Char('p') if ctrl => Some(Msg::Move(Move::Up)),
        KeyCode::Down => Some(Msg::Move(Move::Down)),
        KeyCode::Up => Some(Msg::Move(Move::Up)),
        // 入力中は文字をそのままクエリへ。修飾キー付きは無視する。
        KeyCode::Char(c) if !ctrl => Some(Msg::SearchInput(c)),
        _ => None,
    }
}

fn normal_mode(app: &App, key: KeyEvent, ctrl: bool) -> Option<Msg> {
    match key.code {
        KeyCode::Char('q') => Some(Msg::Quit),
        KeyCode::Char('?') => Some(Msg::ToggleHelp),
        KeyCode::Char('/') => Some(Msg::SearchStart),
        KeyCode::Char('r') => Some(Msg::Reload),
        KeyCode::Char('a') => Some(Msg::ToggleAllNotes),
        KeyCode::Char('1') => Some(Msg::SwitchView(View::Notes)),
        KeyCode::Char('2') => Some(Msg::SwitchView(View::Todo)),
        KeyCode::Char('3') => Some(Msg::SwitchView(View::Projects)),
        KeyCode::Char('4') => Some(Msg::SwitchView(View::Search)),
        KeyCode::Tab => Some(Msg::NextView),
        KeyCode::BackTab => Some(Msg::PrevView),

        KeyCode::Char('d') if ctrl => Some(Msg::Move(Move::PageDown)),
        KeyCode::Char('u') if ctrl => Some(Msg::Move(Move::PageUp)),
        KeyCode::PageDown => Some(Msg::Move(Move::PageDown)),
        KeyCode::PageUp => Some(Msg::Move(Move::PageUp)),
        KeyCode::Char('j') | KeyCode::Down => Some(Msg::Move(Move::Down)),
        KeyCode::Char('k') | KeyCode::Up => Some(Msg::Move(Move::Up)),
        KeyCode::Char('g') | KeyCode::Home => Some(Msg::Move(Move::Top)),
        KeyCode::Char('G') | KeyCode::End => Some(Msg::Move(Move::Bottom)),

        // h/l は本文ペインとの行き来。ノートビューだけ意味を持つ。
        KeyCode::Char('l') | KeyCode::Right if app.focus == Focus::List => Some(Msg::ToggleFocus),
        KeyCode::Char('h') | KeyCode::Left if app.focus == Focus::Detail => Some(Msg::ToggleFocus),

        KeyCode::Char(' ') => Some(Msg::CycleTodo),
        KeyCode::Enter => match app.view {
            View::Search => Some(Msg::SearchCommit),
            View::Notes => Some(Msg::ToggleFocus),
            View::Todo => Some(Msg::CycleTodo),
            View::Projects => None,
        },
        KeyCode::Esc => Some(Msg::DismissStatus),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;
    use std::path::PathBuf;

    fn app() -> App {
        let config = Config {
            data_dir: PathBuf::from("/nonexistent"),
            source: "test".into(),
            recent_notes: 30,
        };
        App::new(config).expect("空のディレクトリでも起動できる")
    }

    fn press(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn press_ctrl(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::CONTROL)
    }

    #[test]
    fn ctrl_c_quits_from_any_mode() {
        let mut a = app();
        a.mode = Mode::Search;
        assert!(matches!(
            handle(&a, press_ctrl(KeyCode::Char('c'))),
            Some(Msg::Quit)
        ));
    }

    #[test]
    fn typing_in_search_mode_feeds_the_query() {
        let mut a = app();
        a.mode = Mode::Search;
        assert!(matches!(
            handle(&a, press(KeyCode::Char('j'))),
            Some(Msg::SearchInput('j'))
        ));
    }

    #[test]
    fn j_navigates_in_normal_mode() {
        let a = app();
        assert!(matches!(
            handle(&a, press(KeyCode::Char('j'))),
            Some(Msg::Move(Move::Down))
        ));
    }

    #[test]
    fn help_mode_exits_on_any_key() {
        let mut a = app();
        a.mode = Mode::Help;
        assert!(matches!(
            handle(&a, press(KeyCode::Char('x'))),
            Some(Msg::ToggleHelp)
        ));
        assert!(matches!(
            handle(&a, press(KeyCode::Char('q'))),
            Some(Msg::Quit)
        ));
    }

    /// フォーカスの向きと逆方向のキーは何もしない。
    #[test]
    fn focus_keys_respect_current_side() {
        let mut a = app();
        a.focus = Focus::List;
        assert!(handle(&a, press(KeyCode::Char('h'))).is_none());
        assert!(matches!(
            handle(&a, press(KeyCode::Char('l'))),
            Some(Msg::ToggleFocus)
        ));
        a.focus = Focus::Detail;
        assert!(matches!(
            handle(&a, press(KeyCode::Char('h'))),
            Some(Msg::ToggleFocus)
        ));
        assert!(handle(&a, press(KeyCode::Char('l'))).is_none());
    }

    #[test]
    fn every_help_row_is_filled() {
        assert!(HELP.iter().all(|(k, d)| !k.is_empty() && !d.is_empty()));
    }
}
