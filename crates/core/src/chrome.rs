//! 聞き取りのページを Google Chrome で開く（DESIGN.md 9.6）。
//!
//! 操作パネルは OBS のドックやアプリの WebView の中にあり、そこから開いても Chrome にならないので、
//! core が Chrome を起動する。ふだんの Chrome のタブとして開く

use std::io;
use std::process::Stdio;

use tokio::process::Command;

/// Chrome が見つからなければ `NotFound`
pub fn open(url: &str) -> io::Result<()> {
    command(url)?
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()?;
    Ok(())
}

#[cfg(target_os = "windows")]
fn command(url: &str) -> io::Result<Command> {
    let chrome = ["ProgramFiles", "ProgramFiles(x86)", "LOCALAPPDATA"]
        .iter()
        .filter_map(std::env::var_os)
        .map(|base| std::path::PathBuf::from(base).join(r"Google\Chrome\Application\chrome.exe"))
        .find(|path| path.exists())
        .ok_or(io::ErrorKind::NotFound)?;
    let mut command = Command::new(chrome);
    command.arg(url);
    Ok(command)
}

#[cfg(target_os = "macos")]
fn command(url: &str) -> io::Result<Command> {
    if !std::path::Path::new("/Applications/Google Chrome.app").exists() {
        return Err(io::ErrorKind::NotFound.into());
    }
    let mut command = Command::new("open");
    command.args(["-a", "Google Chrome", url]);
    Ok(command)
}

#[cfg(not(any(target_os = "windows", target_os = "macos")))]
fn command(url: &str) -> io::Result<Command> {
    let mut command = Command::new("google-chrome");
    command.arg(url);
    Ok(command)
}
