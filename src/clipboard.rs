//! クリップボードへのコピー。
//!
//! 外部コマンドに渡すだけにして、依存を増やさない。macOS の `pbcopy` を先に見て、
//! 無ければ Linux の定番を順に試す。どれも無ければ、その旨を返す。

use std::io::Write;
use std::process::{Command, Stdio};

use anyhow::{bail, Context, Result};

/// 候補のコマンド。前から順に試し、最初に起動できたものを使う。
const CANDIDATES: [&[&str]; 4] = [
    &["pbcopy"],
    &["wl-copy"],
    &["xclip", "-selection", "clipboard"],
    &["xsel", "--clipboard", "--input"],
];

pub fn copy(text: &str) -> Result<&'static str> {
    for candidate in CANDIDATES {
        match feed(candidate, text) {
            Ok(()) => return Ok(candidate[0]),
            // そのコマンドが無いだけなら次を試す。それ以外は理由を伝えて止める。
            Err(err) if is_missing(&err) => continue,
            Err(err) => return Err(err),
        }
    }
    bail!("クリップボードにコピーするコマンドが見つかりません（pbcopy / wl-copy / xclip / xsel）")
}

fn feed(command: &[&str], text: &str) -> Result<()> {
    let mut child = Command::new(command[0])
        .args(&command[1..])
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()?;
    child
        .stdin
        .take()
        .context("標準入力を開けません")?
        .write_all(text.as_bytes())
        .with_context(|| format!("{} に書き込めません", command[0]))?;
    let status = child
        .wait()
        .with_context(|| format!("{} の終了を待てません", command[0]))?;
    if !status.success() {
        bail!("{} が異常終了しました", command[0]);
    }
    Ok(())
}

/// そのコマンドが入っていないだけかどうか。
fn is_missing(err: &anyhow::Error) -> bool {
    err.downcast_ref::<std::io::Error>()
        .is_some_and(|e| e.kind() == std::io::ErrorKind::NotFound)
}
