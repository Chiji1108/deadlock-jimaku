//! 無料モードのページを Chrome で開く（DESIGN.md 9.6）。
//!
//! 全画面のゲームに隠れても認識が止まらないよう、省電力の働きを切った Chrome を専用のプロファイルで起動する。
//! フラグは Chrome の起動時にしか効かないので、ふだんの Chrome のプロファイルとは分ける

use std::io;
use std::path::Path;
use std::process::{Command, Stdio};

const FLAGS: [&str; 6] = [
    "--disable-features=CalculateNativeWinOcclusion,UseEcoQoSForBackgroundProcess",
    "--disable-background-timer-throttling",
    "--disable-backgrounding-occluded-windows",
    "--disable-renderer-backgrounding",
    "--no-first-run",
    "--no-default-browser-check",
];

fn arguments(profile: &Path, url: &str) -> Vec<String> {
    FLAGS
        .iter()
        .map(|&flag| flag.to_owned())
        .chain([
            format!("--user-data-dir={}", profile.display()),
            "--new-window".to_owned(),
            url.to_owned(),
        ])
        .collect()
}

/// Chrome が見つからなければ `NotFound`
pub fn open(data_dir: &Path, url: &str) -> io::Result<()> {
    let args = arguments(&data_dir.join("chrome-profile"), url);
    command(&args)?
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map(drop)
}

#[cfg(target_os = "windows")]
fn command(args: &[String]) -> io::Result<Command> {
    let chrome = ["ProgramFiles", "ProgramFiles(x86)", "LOCALAPPDATA"]
        .iter()
        .filter_map(std::env::var_os)
        .map(|base| std::path::PathBuf::from(base).join(r"Google\Chrome\Application\chrome.exe"))
        .find(|path| path.exists())
        .ok_or(io::ErrorKind::NotFound)?;
    let mut command = Command::new(chrome);
    command.args(args);
    Ok(command)
}

#[cfg(target_os = "macos")]
fn command(args: &[String]) -> io::Result<Command> {
    let app = std::path::PathBuf::from("/Applications/Google Chrome.app");
    if !app.exists() {
        return Err(io::ErrorKind::NotFound.into());
    }
    let mut command = Command::new("open");
    command.arg("-na").arg(app).arg("--args").args(args);
    Ok(command)
}

#[cfg(not(any(target_os = "windows", target_os = "macos")))]
fn command(args: &[String]) -> io::Result<Command> {
    let mut command = Command::new("google-chrome");
    command.args(args);
    Ok(command)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn opens_the_page_last_with_its_own_profile() {
        let args = arguments(
            Path::new("/data/chrome-profile"),
            "http://127.0.0.1:4510/chrome",
        );
        assert_eq!(args.last().unwrap(), "http://127.0.0.1:4510/chrome");
        assert!(args.contains(&"--user-data-dir=/data/chrome-profile".to_owned()));
    }
}
