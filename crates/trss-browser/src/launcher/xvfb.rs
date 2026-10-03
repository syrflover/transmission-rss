//! Keeping the virtual display up.
//!
//! The launcher starts Xvfb itself and starts it again when it dies. A
//! Chromium that was open on a display that died exits by itself, and the
//! launcher then ends its run.

use std::{path::PathBuf, process::Stdio, time::Duration};

use tokio::process::Command;
use tokio_util::sync::CancellationToken;

/// The number of an X display name (`:99` is 99).
fn display_number(display: &str) -> Option<u32> {
    display.strip_prefix(':')?.split('.').next()?.parse().ok()
}

/// The socket the X server of `display` listens on.
pub fn socket_path(display: &str) -> Option<PathBuf> {
    Some(PathBuf::from(format!(
        "/tmp/.X11-unix/X{}",
        display_number(display)?
    )))
}

/// A leftover lock file of an X server that is gone would make the next one
/// refuse to start (the container's folders outlive a restart of it, its
/// processes do not).
fn clear_stale_files(display: &str) {
    if let Some(number) = display_number(display) {
        let _ = std::fs::remove_file(format!("/tmp/.X{number}-lock"));
    }
    if let Some(socket) = socket_path(display) {
        let _ = std::fs::remove_file(socket);
    }
}

/// Runs Xvfb for `display` with a screen of `screen` (`1920x1200x24`) until
/// `cancel` fires, starting it again a second after it exits.
pub async fn supervise(display: String, screen: String, cancel: CancellationToken) {
    loop {
        clear_stale_files(&display);
        let spawned = Command::new("Xvfb")
            .arg(&display)
            .args(["-screen", "0", &screen, "-nolisten", "tcp"])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .kill_on_drop(true)
            .spawn();
        match spawned {
            Ok(mut child) => {
                println!("Xvfb {display} started ({screen})");
                tokio::select! {
                    status = child.wait() => match status {
                        Ok(status) => eprintln!("Xvfb {display} exited ({status}); starting it again"),
                        Err(err) => eprintln!("Xvfb {display}: cannot wait for it: {err}"),
                    },
                    _ = cancel.cancelled() => {
                        let _ = child.kill().await;
                        return;
                    }
                }
            }
            Err(err) => eprintln!("Xvfb {display}: cannot start it: {err}"),
        }
        tokio::select! {
            _ = cancel.cancelled() => return,
            _ = tokio::time::sleep(Duration::from_secs(1)) => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn it_names_the_socket_of_a_display() {
        assert_eq!(
            socket_path(":99").as_deref(),
            Some(std::path::Path::new("/tmp/.X11-unix/X99"))
        );
        assert_eq!(
            socket_path(":7.0").as_deref(),
            Some(std::path::Path::new("/tmp/.X11-unix/X7"))
        );
        assert_eq!(socket_path("host:1"), None);
        assert_eq!(socket_path("99"), None);
    }
}
