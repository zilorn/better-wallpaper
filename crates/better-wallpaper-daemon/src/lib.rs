//! daemon 的可复用运行时组件。

pub mod plasma_frames;
pub mod plasma_audio;
pub mod playback;
pub mod server;
pub mod tray;

use std::{
    collections::VecDeque,
    io::Write,
    sync::{Arc, Mutex},
};

/// 环形日志缓冲区，用于在 Web UI 中预览日志。
#[derive(Clone)]
pub struct LogStore {
    inner: Arc<Mutex<LogBuffer>>,
}

struct LogBuffer {
    lines: VecDeque<String>,
    max: usize,
}

impl LogStore {
    pub fn new(max: usize) -> Self {
        Self {
            inner: Arc::new(Mutex::new(LogBuffer {
                lines: VecDeque::with_capacity(max.min(64)),
                max,
            })),
        }
    }

    pub fn push(&self, line: String) {
        if let Ok(mut buf) = self.inner.lock() {
            if buf.lines.len() >= buf.max {
                buf.lines.pop_front();
            }
            buf.lines.push_back(line);
        }
    }

    pub fn lines(&self) -> Vec<String> {
        self.inner
            .lock()
            .map(|buf| buf.lines.iter().cloned().collect())
            .unwrap_or_default()
    }
}

impl tracing_subscriber::fmt::MakeWriter<'_> for LogStore {
    type Writer = LogWriter;
    fn make_writer(&self) -> Self::Writer {
        LogWriter {
            store: self.clone(),
        }
    }
}

pub struct LogWriter {
    store: LogStore,
}

impl Write for LogWriter {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        // Strip ANSI escape sequences for the LogStore
        let cleaned = strip_ansi_escapes(buf);
        if !cleaned.trim().is_empty() {
            for line in cleaned.lines() {
                self.store.push(line.to_string());
            }
        }
        // Write original bytes (with ANSI colors) to stderr
        std::io::stderr().write(buf)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        std::io::stderr().flush()
    }
}

/// Remove ANSI escape sequences (SGR codes like ESC[...m) from log output.
fn strip_ansi_escapes(buf: &[u8]) -> String {
    let s = String::from_utf8_lossy(buf);
    let mut result = String::with_capacity(s.len());
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c == '\x1b' || c == '\u{001b}' {
            // ESC character - skip CSI sequences: ESC[...m or ESC[K
            // Also handles OSC sequences: ESC]...BEL
            while let Some(n) = chars.next() {
                if n == 'm' || n == 'K' || n == '\x07' {
                    break;
                }
            }
        } else {
            result.push(c);
        }
    }
    result
}

