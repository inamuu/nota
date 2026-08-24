# nota

[Acta](https://github.com/inamuu/Acta)（デスクトップのメモアプリ）のデータを、ターミナルから読むための TUI。

操作はすべてキーボードで完結する。マウスは使わない。

## できること

現時点は閲覧が中心で、書き込みは ToDo のチェックだけに絞っている。

| ビュー | 内容 |
| --- | --- |
| ノート | デイリーノートを日付降順に一覧し、選んだ日のエントリを本文ペインに表示。既定は直近 30 件で、`a` で全件に切り替え |
| ToDo | 全ノートから ToDo のタスク行を集めて一覧。Space で状態を進める |
| プロジェクト | プロジェクト一覧と、タスクを状態別に表示 |
| 検索 | 全エントリを対象にインクリメンタル検索。Enter で該当箇所へジャンプ |

Acta とは同じファイルを読む。nota が書き込むのは ToDo のチェック欄 1 文字だけで、
それ以外の行には触らない。読み込んだ内容は原文のまま保持し、書き戻しでは
該当行を差し替えて再結合するので、空行や記法の揺れは保たれる。

## インストール

Rust ツールチェーンは [mise](https://mise.jdx.dev/) で管理している。

```sh
mise install
mise run build          # cargo build
mise run release        # cargo build --release
```

バイナリは `target/release/nota`。

## 設定

Acta のデータディレクトリを教える必要がある。パスはローカル固有の情報なので、
リポジトリには含めない。

推奨は `~/.config/nota/config.toml` に置く方法。

```toml
data_dir = "/path/to/your/Acta"

# ノート一覧に最初から出す件数。既定は 30。0 なら最初から全件。
recent_notes = 30
```

リポジトリ内に置きたい場合は `config.local.toml`（gitignore 済み）を使う。

```sh
cp config.example.toml config.local.toml
```

探索順は次のとおりで、最初に見つかったものを使う。`posts/` か `projects/` を
含むディレクトリでなければ次の候補へ進む。

1. `--data-dir <PATH>`
2. 環境変数 `NOTA_DATA_DIR`
3. 環境変数 `NOTA_CONFIG` が指すファイルの `data_dir`
4. `./config.local.toml` の `data_dir`
5. `~/.config/nota/config.toml` の `data_dir`
6. `~/Documents/Acta`

どこから読まれたかは `?` のヘルプ画面に出る。

## 起動

```sh
mise run run                        # cargo run
nota                                # 設定ファイルから解決
nota --data-dir /path/to/Acta       # 直接指定
```

## キー操作

`?` でいつでも一覧を表示できる。

| キー | 動作 |
| --- | --- |
| `j` / `k`, `↓` / `↑` | 選択を上下に移動 |
| `Ctrl-d` / `Ctrl-u` | 半画面ずつ移動 |
| `g` / `G` | 先頭 / 末尾へ |
| `1` / `2` / `3` / `4` | ノート / ToDo / プロジェクト / 検索 |
| `Tab` / `Shift-Tab` | ビューを順に切り替え |
| `h` / `l` | 一覧と本文のフォーカスを移動 |
| `Enter` | ノートは本文へ、検索は該当箇所へジャンプ |
| `Space` | ToDo の状態を進める（未着手 → 進行中 → 完了） |
| `/` | 全文検索 |
| `a` | ノート一覧を直近だけ / 全件で切り替え |
| `r` | データを再読み込み |
| `?` | ヘルプ |
| `q`, `Ctrl-c` | 終了 |
| `Esc` | モードを抜ける / メッセージを消す |

## 開発

```sh
mise run test           # cargo test
mise run lint           # clippy + fmt --check
mise run fmt            # cargo fmt
```

実データに対する健全性チェックは `#[ignore]` にしてある。全ノートで原文を
復元できるか（書き戻しの安全性）を確かめるので、データ形式に手を入れたときは
これを通す。

```sh
NOTA_DATA_DIR=/path/to/Acta cargo test -- --ignored --nocapture
```

## 構成

| ファイル | 役割 |
| --- | --- |
| `src/model.rs` | Acta のファイル形式。原文を保持したままパースする |
| `src/store.rs` | ディレクトリ走査と読み書き |
| `src/config.rs` | データディレクトリの解決 |
| `src/app.rs` | 状態と更新（`App` と `Msg`） |
| `src/keys.rs` | キー入力から `Msg` への変換 |
| `src/ui.rs` | 描画。状態は変えない |
| `src/smoke.rs` | 描画と実データ読み込みの検証 |

状態は `App` に集約し、変更は必ず `Msg` を通す。描画は `App` を読むだけ。
編集機能を足すときは `Msg` を増やして `update` に分岐を加える。

## 読んでいるデータ形式

Acta が書き出す Markdown をそのまま読む。

```
posts/YYYY/MM/DD/YYYY-MM-DD.md     デイリーノート
projects/<dir>/project.json        プロジェクトとタスク
projects/<dir>/knowledge.md        プロジェクトのナレッジ
```

デイリーノートの中身は次の形。

```markdown
# 2026-02-26

<!-- acta:comment
id: d1d0d072-1dae-4a86-ae77-5b100373d3ad
created: 2026-02-26 11:25
created_ms: 1772072708000
tags: ToDo, Terraform
-->
# ToDo: 2026/02/26（木）
- プロジェクト名
  - [ ] 未着手
  - [-] 進行中
  - [x] 完了
<!-- /acta:comment -->
```

ToDo として扱うのは、`tags` に `ToDo` を含むエントリか、本文が `# ToDo` で
始まるエントリ。チェック欄の `[ ]` / `[-]` / `[x]` が Backlog / InProgress /
Done に対応する。
