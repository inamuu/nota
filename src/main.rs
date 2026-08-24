//! nota — Acta のデータをターミナルから読む TUI。
//!
//! 操作はすべてキーボードで完結する。マウスは有効化しない。

mod app;
mod config;
mod editor;
mod keys;
mod model;
mod smoke;
mod store;
mod ui;

use std::io::{self, Stdout};
use std::time::Duration;

use anyhow::{Context, Result};
use crossterm::event::{self, Event, KeyEventKind};
use crossterm::execute;
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use ratatui::backend::CrosstermBackend;
use ratatui::Terminal;

use app::{App, Msg};
use config::Config;
use editor::EditRequest;

const USAGE: &str = "\
nota — Acta のデータをターミナルから読む TUI

使い方:
    nota [オプション]

オプション:
    --data-dir <PATH>  Acta のデータディレクトリを指定する
    -h, --help         このヘルプを表示する
    -V, --version      バージョンを表示する

データディレクトリの探索順:
    1. --data-dir
    2. 環境変数 NOTA_DATA_DIR
    3. 環境変数 NOTA_CONFIG が指すファイルの data_dir
    4. ./config.local.toml の data_dir
    5. ~/.config/nota/config.toml の data_dir
    6. ~/Documents/Acta
";

fn main() -> Result<()> {
    let mut data_dir: Option<String> = None;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "-h" | "--help" => {
                print!("{USAGE}");
                return Ok(());
            }
            "-V" | "--version" => {
                println!("nota {}", env!("CARGO_PKG_VERSION"));
                return Ok(());
            }
            "--data-dir" => {
                data_dir = Some(args.next().context("--data-dir にパスを指定してください")?);
            }
            other => {
                anyhow::bail!("不明な引数です: {other}\n\n{USAGE}");
            }
        }
    }

    let config = Config::load(data_dir.as_deref())?;
    let mut app = App::new(config)?;

    let mut terminal = setup()?;
    // パニックしても端末を元に戻す。戻さないと以降の操作が壊れる。
    let hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let _ = restore();
        hook(info);
    }));

    let result = run(&mut terminal, &mut app);
    restore()?;
    result
}

/// $EDITOR を起動する。端末の制御を一度返し、戻ってきたら画面を作り直す。
/// エディタが失敗しても TUI に戻れるよう、エラーは状態行に出すだけにする。
fn edit(
    terminal: &mut Terminal<CrosstermBackend<Stdout>>,
    app: &mut App,
    request: EditRequest,
) -> Result<()> {
    restore()?;
    let result = editor::run(&request.initial, "edit");
    enable_raw_mode()?;
    execute!(io::stdout(), EnterAlternateScreen)?;
    terminal.clear()?;

    match result {
        Ok(edited) => app.apply_edit(request.target, edited),
        Err(err) => app.report(format!("{err}")),
    }
    Ok(())
}

fn setup() -> Result<Terminal<CrosstermBackend<Stdout>>> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let terminal = Terminal::new(CrosstermBackend::new(stdout))?;
    Ok(terminal)
}

fn restore() -> Result<()> {
    disable_raw_mode()?;
    execute!(io::stdout(), LeaveAlternateScreen)?;
    Ok(())
}

fn run(terminal: &mut Terminal<CrosstermBackend<Stdout>>, app: &mut App) -> Result<()> {
    loop {
        terminal.draw(|frame| ui::draw(app, frame))?;

        // エディタを開く要求があれば、端末を明け渡してから起動する。
        if let Some(request) = app.take_edit_request() {
            edit(terminal, app, request)?;
            continue;
        }

        // ポーリングにしておくと、端末リサイズなども取りこぼさない。
        if !event::poll(Duration::from_millis(200))? {
            continue;
        }
        match event::read()? {
            Event::Key(key) if key.kind == KeyEventKind::Press => {
                if let Some(msg) = keys::handle(app, key) {
                    let quit = matches!(msg, Msg::Quit);
                    app.update(msg);
                    if quit || app.should_quit {
                        return Ok(());
                    }
                }
            }
            _ => {}
        }
    }
}
