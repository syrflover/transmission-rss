//! A server on a loopback address for a test: the bind, the address and the
//! task, whatever the server speaks. What it serves (an axum `Router`, a loop
//! that accepts raw connections) is the caller's closure, so this module needs
//! no HTTP framework; and the closure keeps what the test relies on: whether
//! a failure of the server panics its task (`unwrap`) or ends it quietly
//! (`ok`), and whether the test keeps or aborts the handle.

use std::{
    future::Future,
    net::{Ipv4Addr, SocketAddr},
};

use tokio::{net::TcpListener, task::JoinHandle};

/// A running server: where it listens and the task that runs it. Dropping
/// this leaves the task running; abort it where a test stops the server.
pub struct Served {
    pub addr: SocketAddr,
    pub task: JoinHandle<()>,
}

/// A bound listener nothing serves yet, for a server whose routes or state
/// need its address before it starts.
pub struct Bound {
    pub addr: SocketAddr,
    listener: TcpListener,
}

impl Bound {
    /// Spawns `run` with the listener.
    pub fn spawn<F, Fut>(self, run: F) -> Served
    where
        F: FnOnce(TcpListener) -> Fut,
        Fut: Future<Output = ()> + Send + 'static,
    {
        Served {
            addr: self.addr,
            task: tokio::spawn(run(self.listener)),
        }
    }
}

/// Binds `127.0.0.1:0`.
pub async fn bind() -> Bound {
    bind_on(Ipv4Addr::LOCALHOST).await
}

/// Binds a free port of `ip`: another loopback address, or `0.0.0.0` for a
/// server a container reaches.
pub async fn bind_on(ip: Ipv4Addr) -> Bound {
    let listener = TcpListener::bind((ip, 0))
        .await
        .expect("bind a test server");
    Bound {
        addr: listener.local_addr().unwrap(),
        listener,
    }
}

/// Binds `127.0.0.1:0` and spawns `run` with the listener.
pub async fn serve<F, Fut>(run: F) -> Served
where
    F: FnOnce(TcpListener) -> Fut,
    Fut: Future<Output = ()> + Send + 'static,
{
    bind().await.spawn(run)
}

/// [`serve`] on a free port of `ip`.
pub async fn serve_on<F, Fut>(ip: Ipv4Addr, run: F) -> Served
where
    F: FnOnce(TcpListener) -> Fut,
    Fut: Future<Output = ()> + Send + 'static,
{
    bind_on(ip).await.spawn(run)
}

/// An address on `127.0.0.1` that nothing listens on: a free port, reserved
/// and released at once. Another process could take it in between, which a
/// test of a refused connection accepts.
pub fn unused_addr() -> SocketAddr {
    std::net::TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
        .expect("bind a test address")
        .local_addr()
        .unwrap()
}

#[cfg(test)]
mod tests {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    use super::*;

    async fn echo_once(listener: TcpListener) {
        let (mut stream, _) = listener.accept().await.unwrap();
        let mut byte = [0];
        stream.read_exact(&mut byte).await.unwrap();
        stream.write_all(&[byte[0] + 1]).await.unwrap();
    }

    #[tokio::test]
    async fn serve_listens_on_the_loopback_address_and_runs_the_closure_with_the_listener() {
        let served = serve(echo_once).await;
        assert_eq!(served.addr.ip(), Ipv4Addr::LOCALHOST);
        assert_ne!(served.addr.port(), 0);
        let mut client = tokio::net::TcpStream::connect(served.addr).await.unwrap();
        client.write_all(&[41]).await.unwrap();
        let mut answer = [0];
        client.read_exact(&mut answer).await.unwrap();
        assert_eq!(answer, [42]);
        served.task.await.unwrap();
    }

    #[tokio::test]
    async fn serve_on_binds_the_address_it_is_given() {
        let other = Ipv4Addr::new(127, 0, 0, 2);
        let served = serve_on(other, echo_once).await;
        assert_eq!(served.addr.ip(), other);
        assert!(tokio::net::TcpStream::connect(served.addr).await.is_ok());
        served.task.abort();
    }

    #[tokio::test]
    async fn a_bound_listener_has_its_address_before_anything_serves_it() {
        let bound = bind().await;
        let addr = bound.addr;
        // Bound, so a connection waits in the backlog until the server runs.
        let mut client = tokio::net::TcpStream::connect(addr).await.unwrap();
        client.write_all(&[1]).await.unwrap();
        let served = bound.spawn(echo_once);
        assert_eq!(served.addr, addr);
        let mut answer = [0];
        client.read_exact(&mut answer).await.unwrap();
        assert_eq!(answer, [2]);
    }

    #[tokio::test]
    async fn servers_started_together_get_different_ports() {
        let first = serve(echo_once).await;
        let second = serve(echo_once).await;
        assert_ne!(first.addr, second.addr);
        first.task.abort();
        second.task.abort();
    }

    #[tokio::test]
    async fn an_aborted_server_stops_listening() {
        let served = serve(|listener| async move {
            let _listener = listener;
            std::future::pending::<()>().await;
        })
        .await;
        served.task.abort();
        assert!(served.task.await.unwrap_err().is_cancelled());
        // The listener went with the task.
        assert!(tokio::net::TcpStream::connect(served.addr).await.is_err());
    }

    #[tokio::test]
    async fn unused_addr_is_a_loopback_address_nothing_answers_on() {
        let addr = unused_addr();
        assert_eq!(addr.ip(), Ipv4Addr::LOCALHOST);
        assert_ne!(addr.port(), 0);
        assert!(tokio::net::TcpStream::connect(addr).await.is_err());
    }
}
