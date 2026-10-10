use std::io::IsTerminal;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StreamType {
    Pipe,
    File,
    Tty,
    Null,
}

impl StreamType {
    pub fn description(&self) -> &'static str {
        match self {
            StreamType::Pipe => "a pipe from another command",
            StreamType::File => "a redirected file",
            StreamType::Tty => "an interactive terminal",
            StreamType::Null => "empty (no input)",
        }
    }

    pub fn is_readable(&self) -> bool {
        matches!(self, StreamType::Pipe | StreamType::File)
    }
}

#[derive(Debug, Clone)]
pub struct Environment {
    pub stdin_type: StreamType,
    pub stdout_type: StreamType,
}

impl Environment {
    pub fn detect() -> Self {
        Environment {
            stdin_type: detect_stdin(),
            stdout_type: detect_stdout(),
        }
    }
}

#[cfg(unix)]
fn detect_stdin() -> StreamType {
    use std::os::unix::io::AsRawFd;

    let stdin = std::io::stdin();
    if stdin.is_terminal() {
        return StreamType::Tty;
    }

    let fd = stdin.as_raw_fd();
    detect_fd_type(fd)
}

#[cfg(unix)]
fn detect_stdout() -> StreamType {
    use std::os::unix::io::AsRawFd;

    let stdout = std::io::stdout();
    if stdout.is_terminal() {
        return StreamType::Tty;
    }

    let fd = stdout.as_raw_fd();
    detect_fd_type(fd)
}

#[cfg(unix)]
fn detect_fd_type(fd: i32) -> StreamType {
    use std::os::unix::fs::FileTypeExt;

    // Try to get metadata via /proc/self/fd/N
    let path = format!("/proc/self/fd/{}", fd);
    if let Ok(meta) = std::fs::metadata(&path) {
        let ft = meta.file_type();
        if ft.is_fifo() {
            return StreamType::Pipe;
        }
        if ft.is_file() {
            return StreamType::File;
        }
        if ft.is_char_device() {
            return StreamType::Null;
        }
    }

    // Fallback: assume pipe
    StreamType::Pipe
}

#[cfg(not(unix))]
fn detect_stdin() -> StreamType {
    if std::io::stdin().is_terminal() {
        StreamType::Tty
    } else {
        StreamType::Pipe
    }
}

#[cfg(not(unix))]
fn detect_stdout() -> StreamType {
    if std::io::stdout().is_terminal() {
        StreamType::Tty
    } else {
        StreamType::Pipe
    }
}
