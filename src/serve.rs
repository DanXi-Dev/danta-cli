use anyhow::{Context, Result};
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::process::Stdio;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

pub async fn run(bind: &str, port: u16) -> Result<()> {
    let addr = format!("{}:{}", bind, port);
    let listener = TcpListener::bind(&addr).await?;

    if bind != "127.0.0.1" && bind != "::1" && bind != "localhost" {
        eprintln!(
            "WARNING: Binding to {} — connected clients will use the server's saved auth token.",
            bind
        );
    }

    eprintln!("Danta TUI server listening on {}", addr);
    eprintln!("Connect with: nc {} {} or telnet {} {}", bind, port, bind, port);

    loop {
        let (stream, peer) = listener.accept().await?;
        eprintln!("[+] Connection from {}", peer);

        tokio::spawn(async move {
            if let Err(e) = handle_connection(stream, peer).await {
                eprintln!("[-] Error for {}: {}", peer, e);
            }
            eprintln!("[-] Disconnected: {}", peer);
        });
    }
}

/// Open a pseudo-terminal pair and return (master_fd, slave_fd)
fn open_pty() -> Result<(OwnedFd, OwnedFd)> {
    let mut master: libc::c_int = 0;
    let mut slave: libc::c_int = 0;

    let ret = unsafe {
        libc::openpty(
            &mut master,
            &mut slave,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
        )
    };

    if ret != 0 {
        anyhow::bail!("openpty failed: {}", std::io::Error::last_os_error());
    }

    let master_fd = unsafe { OwnedFd::from_raw_fd(master) };
    let slave_fd = unsafe { OwnedFd::from_raw_fd(slave) };

    Ok((master_fd, slave_fd))
}

/// Set PTY window size
fn set_pty_size(master_fd: &OwnedFd, cols: u16, rows: u16) {
    let ws = libc::winsize {
        ws_row: rows,
        ws_col: cols,
        ws_xpixel: 0,
        ws_ypixel: 0,
    };
    unsafe {
        libc::ioctl(master_fd.as_raw_fd(), libc::TIOCSWINSZ, &ws);
    }
}

async fn handle_connection(
    stream: tokio::net::TcpStream,
    peer: std::net::SocketAddr,
) -> Result<()> {
    // Allocate a PTY pair
    let (master_fd, slave_fd) = open_pty().context("Failed to allocate PTY")?;

    // Set a reasonable default terminal size
    set_pty_size(&master_fd, 80, 24);

    // The slave side becomes the child process's stdin/stdout/stderr
    let slave_for_stdin = unsafe { OwnedFd::from_raw_fd(libc::dup(slave_fd.as_raw_fd())) };
    let slave_for_stdout = unsafe { OwnedFd::from_raw_fd(libc::dup(slave_fd.as_raw_fd())) };
    let slave_for_stderr = slave_fd; // last one takes ownership

    let exe = std::env::current_exe()?;

    let mut child = tokio::process::Command::new(exe)
        .arg("tui")
        .env("TERM", "xterm-256color")
        .stdin(Stdio::from(slave_for_stdin))
        .stdout(Stdio::from(slave_for_stdout))
        .stderr(Stdio::from(slave_for_stderr))
        .spawn()
        .context("Failed to spawn TUI child process")?;

    // Bridge TCP stream <-> PTY master
    // We wrap the master fd in a tokio AsyncFd for non-blocking I/O
    let master_raw = master_fd.as_raw_fd();

    // Set master fd to non-blocking
    unsafe {
        let flags = libc::fcntl(master_raw, libc::F_GETFL);
        libc::fcntl(master_raw, libc::F_SETFL, flags | libc::O_NONBLOCK);
    }

    let master_async = tokio::io::unix::AsyncFd::new(master_fd)?;
    let (tcp_read, tcp_write) = stream.into_split();

    // Spawn two tasks: TCP->PTY and PTY->TCP
    let tcp_to_pty = tokio::spawn(bridge_tcp_to_pty(tcp_read, master_async.as_raw_fd()));
    let pty_to_tcp = tokio::spawn(bridge_pty_to_tcp(master_async, tcp_write));

    // Wait for child or bridge to finish
    tokio::select! {
        status = child.wait() => {
            eprintln!("[*] {} child exited with {}", peer, status?);
        }
        _ = tcp_to_pty => {}
        _ = pty_to_tcp => {}
    }

    // Clean up
    child.kill().await.ok();

    Ok(())
}

async fn bridge_tcp_to_pty(
    mut tcp_read: tokio::net::tcp::OwnedReadHalf,
    master_raw_fd: std::os::fd::RawFd,
) -> Result<()> {
    let mut buf = [0u8; 4096];
    loop {
        let n = tcp_read.read(&mut buf).await?;
        if n == 0 {
            break; // TCP connection closed
        }
        // Write to PTY master (blocking write is fine for small chunks)
        let written = unsafe {
            libc::write(
                master_raw_fd,
                buf[..n].as_ptr() as *const libc::c_void,
                n,
            )
        };
        if written < 0 {
            break;
        }
    }
    Ok(())
}

async fn bridge_pty_to_tcp(
    master_async: tokio::io::unix::AsyncFd<OwnedFd>,
    mut tcp_write: tokio::net::tcp::OwnedWriteHalf,
) -> Result<()> {
    let mut buf = [0u8; 4096];
    loop {
        // Wait for the PTY master to be readable
        let mut guard = master_async.readable().await?;

        match guard.try_io(|inner| {
            let fd = inner.as_raw_fd();
            let n = unsafe {
                libc::read(fd, buf.as_mut_ptr() as *mut libc::c_void, buf.len())
            };
            if n < 0 {
                Err(std::io::Error::last_os_error())
            } else {
                Ok(n as usize)
            }
        }) {
            Ok(Ok(0)) => break, // EOF
            Ok(Ok(n)) => {
                tcp_write.write_all(&buf[..n]).await?;
            }
            Ok(Err(e)) => {
                // EIO is expected when the child exits
                if e.raw_os_error() == Some(libc::EIO) {
                    break;
                }
                return Err(e.into());
            }
            Err(_would_block) => continue,
        }
    }
    Ok(())
}
