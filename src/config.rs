//! データディレクトリの解決。
//!
//! 実際のパスはローカル固有の情報なのでリポジトリには含めない。
//! 探索順は README のとおりで、最初に見つかったものを使う。

use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use serde::Deserialize;

const ENV_DATA_DIR: &str = "NOTA_DATA_DIR";
const ENV_CONFIG: &str = "NOTA_CONFIG";
const LOCAL_CONFIG: &str = "config.local.toml";
const USER_CONFIG: &str = "nota/config.toml";
const FALLBACK_DATA_DIR: &str = "Documents/Acta";

#[derive(Debug, Default, Deserialize)]
struct ConfigFile {
    data_dir: Option<String>,
}

#[derive(Debug, Clone)]
pub struct Config {
    pub data_dir: PathBuf,
    /// 設定の出どころ。起動時に画面下部へ出して、どこを直せばよいか分かるようにする。
    pub source: String,
}

impl Config {
    /// コマンドライン引数と環境変数から解決する。
    pub fn load(explicit: Option<&str>) -> Result<Self> {
        let from_env = std::env::var(ENV_DATA_DIR).ok();
        let explicit = explicit.or(from_env.as_deref());
        Self::resolve(explicit)
    }

    /// 解決の本体。環境変数を読まないので、テストから直接呼べる。
    fn resolve(explicit: Option<&str>) -> Result<Self> {
        // 明示指定は必ずそこを使う。外れていたら黙って別の場所へ移らず、
        // 指定が間違っていることを伝える。
        if let Some(dir) = explicit {
            if !dir.trim().is_empty() {
                let expanded = expand_tilde(dir);
                if !looks_like_data_dir(&expanded) {
                    bail!(
                        "指定されたディレクトリに Acta のデータがありません: {}\n\
                         posts/ か projects/ を含むディレクトリを指定してください。",
                        expanded.display()
                    );
                }
                return Ok(Self {
                    data_dir: expanded,
                    source: format!("--data-dir または {ENV_DATA_DIR}"),
                });
            }
        }

        for candidate in Self::candidates()? {
            let (data_dir, source) = candidate;
            let expanded = expand_tilde(&data_dir);
            if looks_like_data_dir(&expanded) {
                return Ok(Self {
                    data_dir: expanded,
                    source,
                });
            }
        }
        bail!(
            "Acta のデータディレクトリが見つかりません。\n\
             次のいずれかで posts/ と projects/ を含むディレクトリを指定してください。\n\
             \n\
             1. 環境変数: export {ENV_DATA_DIR}=/path/to/Acta\n\
             2. 設定ファイル: ~/.config/{USER_CONFIG}\n\
             \n\
             設定ファイルの書式は config.example.toml を参照してください。"
        )
    }

    /// (パス, 出どころ) の候補を優先順に返す。
    fn candidates() -> Result<Vec<(String, String)>> {
        let mut out = Vec::new();

        if let Ok(path) = std::env::var(ENV_CONFIG) {
            let path = PathBuf::from(path);
            if let Some(dir) = read_data_dir(&path)? {
                out.push((dir, format!("{} ({ENV_CONFIG})", path.display())));
            }
        }

        let local = PathBuf::from(LOCAL_CONFIG);
        if let Some(dir) = read_data_dir(&local)? {
            out.push((dir, LOCAL_CONFIG.to_string()));
        }

        for user in user_config_paths() {
            if let Some(dir) = read_data_dir(&user)? {
                out.push((dir, user.display().to_string()));
            }
        }

        if let Some(home) = dirs::home_dir() {
            let fallback = home.join(FALLBACK_DATA_DIR);
            out.push((fallback.display().to_string(), "既定値".to_string()));
        }

        Ok(out)
    }
}

/// Acta のデータディレクトリらしいか。posts/ か projects/ があれば採用する。
fn looks_like_data_dir(dir: &Path) -> bool {
    dir.join("posts").is_dir() || dir.join("projects").is_dir()
}

/// ユーザー設定ファイルの候補を優先順に返す。
///
/// macOS の `dirs::config_dir()` は `~/Library/Application Support` を返すが、
/// CLI ツールの設定は `~/.config` に置くのが慣習なのでそちらを先に見る。
fn user_config_paths() -> Vec<PathBuf> {
    let mut out = Vec::new();

    if let Ok(xdg) = std::env::var("XDG_CONFIG_HOME") {
        if !xdg.trim().is_empty() {
            out.push(expand_tilde(&xdg).join(USER_CONFIG));
        }
    }
    if let Some(home) = dirs::home_dir() {
        out.push(home.join(".config").join(USER_CONFIG));
    }
    // プラットフォーム標準の場所も最後に見る。
    if let Some(config_dir) = dirs::config_dir() {
        out.push(config_dir.join(USER_CONFIG));
    }

    out.dedup();
    out
}

fn read_data_dir(path: &Path) -> Result<Option<String>> {
    if !path.is_file() {
        return Ok(None);
    }
    let text = std::fs::read_to_string(path)
        .with_context(|| format!("設定ファイルを読めません: {}", path.display()))?;
    let parsed: ConfigFile = toml::from_str(&text)
        .with_context(|| format!("設定ファイルの書式が不正です: {}", path.display()))?;
    Ok(parsed
        .data_dir
        .map(|d| d.trim().to_string())
        .filter(|d| !d.is_empty()))
}

fn expand_tilde(raw: &str) -> PathBuf {
    let trimmed = raw.trim();
    if trimmed == "~" {
        if let Some(home) = dirs::home_dir() {
            return home;
        }
    }
    if let Some(rest) = trimmed.strip_prefix("~/") {
        if let Some(home) = dirs::home_dir() {
            return home.join(rest);
        }
    }
    PathBuf::from(trimmed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn expands_bare_tilde() {
        let home = dirs::home_dir().expect("home");
        assert_eq!(expand_tilde("~"), home);
    }

    #[test]
    fn expands_tilde_prefix() {
        let home = dirs::home_dir().expect("home");
        assert_eq!(
            expand_tilde("~/Documents/Acta"),
            home.join("Documents/Acta")
        );
    }

    #[test]
    fn keeps_absolute_path() {
        assert_eq!(expand_tilde("/tmp/Acta"), PathBuf::from("/tmp/Acta"));
    }

    #[test]
    fn trims_surrounding_space() {
        assert_eq!(expand_tilde("  /tmp/Acta  "), PathBuf::from("/tmp/Acta"));
    }

    /// macOS で `~/Library/Application Support` だけを見ていると、
    /// `~/.config` に置いた設定が読まれない。順序を守っていることを確かめる。
    #[test]
    fn prefers_dot_config_over_platform_dir() {
        let paths = user_config_paths();
        let home = dirs::home_dir().expect("home");
        let expected = home.join(".config").join(USER_CONFIG);
        let position = paths.iter().position(|p| *p == expected);
        assert!(position.is_some(), "~/.config が候補に含まれていない");
        if let Some(platform) = dirs::config_dir() {
            let platform = platform.join(USER_CONFIG);
            if let Some(other) = paths.iter().position(|p| *p == platform) {
                if platform != expected {
                    assert!(position.unwrap() < other, "~/.config を先に見ていない");
                }
            }
        }
    }

    /// 明示指定が外れたら、別の場所にフォールバックせずエラーにする。
    /// 黙って別のデータを開くと、どこを見ているのか分からなくなる。
    #[test]
    fn explicit_data_dir_does_not_fall_back() {
        let err = Config::resolve(Some("/nonexistent-acta-data"))
            .expect_err("エラーになる")
            .to_string();
        assert!(
            err.contains("/nonexistent-acta-data"),
            "指定パスを示していない: {err}"
        );
    }

    /// 空文字の指定は「指定なし」として扱い、通常の探索に進む。
    #[test]
    fn blank_explicit_falls_through() {
        // データが見つかるかは環境次第なので、パニックしないことだけを見る。
        let _ = Config::resolve(Some("   "));
    }

    #[test]
    fn missing_config_file_is_none() {
        assert!(read_data_dir(Path::new("/nonexistent/nota.toml"))
            .expect("ok")
            .is_none());
    }
}
