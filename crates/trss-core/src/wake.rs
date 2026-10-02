//! How the web wakes the worker when it has accepted a command.
//!
//! The worker listens on a Unix datagram socket next to the database
//! ([`wake_path_for`]); the web and the worker share the database's folder,
//! so they share the socket's path too. Any datagram means "look for
//! commands now". The worker also looks every few seconds without being
//! woken, so a wake that is lost (no worker listens, the socket is full, the
//! worker is restarting) only delays a command until that look.
//!
//! One worker listens at a time: a worker that starts binds the path anew,
//! and an older worker still running on the same database is then woken only
//! by its own looks.

use std::{
    ffi::OsString,
    os::unix::net::UnixDatagram,
    path::{Path, PathBuf},
};

/// The wake socket's path for a database file: the database path plus
/// `.wake`.
pub fn wake_path_for(db_path: &Path) -> PathBuf {
    let mut name: OsString = db_path.as_os_str().to_owned();
    name.push(".wake");
    PathBuf::from(name)
}

/// Wakes the worker listening at `path`, if any. Never waits and never fails:
/// a worker that is not there looks for commands by itself when it starts.
pub fn wake_worker(path: &Path) {
    let Ok(socket) = UnixDatagram::unbound() else {
        return;
    };
    let _ = socket.set_nonblocking(true);
    let _ = socket.send_to(&[1], path);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_wake_socket_sits_next_to_the_database() {
        assert_eq!(
            wake_path_for(Path::new("/data/trss/app.db")),
            Path::new("/data/trss/app.db.wake")
        );
    }

    #[test]
    fn a_listening_worker_hears_the_wake_and_a_missing_one_is_no_error() {
        let dir = tempfile::tempdir().unwrap();
        let path = wake_path_for(&dir.path().join("app.db"));
        // Nobody listens yet: nothing happens.
        wake_worker(&path);

        let listener = UnixDatagram::bind(&path).unwrap();
        listener.set_nonblocking(true).unwrap();
        wake_worker(&path);
        let mut buf = [0u8; 8];
        assert_eq!(listener.recv(&mut buf).unwrap(), 1);
    }
}
