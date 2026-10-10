//! 串口会话：打开、收发事务（协议 v1，一问一答）。

use std::io;
use std::time::{Duration, Instant};

use crate::protocol::{encode_line, parse_response, Response};

pub const BAUD: u32 = 115_200;
pub const READ_TIMEOUT: Duration = Duration::from_millis(200);

#[derive(Debug, PartialEq)]
pub enum TxError {
    Timeout,
    BadLine,
    Io(String),
}

impl std::fmt::Display for TxError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TxError::Timeout => write!(f, "hub 应答超时"),
            TxError::BadLine => write!(f, "hub 应答格式非法"),
            TxError::Io(e) => write!(f, "串口错误: {e}"),
        }
    }
}

pub struct SerialSession {
    port: Box<dyn serialport::SerialPort>,
    buf: Vec<u8>,
}

impl SerialSession {
    pub fn open(path: &str) -> Result<Self, String> {
        let port = serialport::new(path, BAUD)
            .timeout(READ_TIMEOUT)
            .open()
            .map_err(|e| format!("打开 {path} 失败: {e}"))?;
        Ok(Self { port, buf: Vec::new() })
    }

    /// 发送一条命令并等待应答行（`docs/protocol.md` §2 一问一答）。
    pub fn transact(&mut self, payload: &str) -> Result<Response, TxError> {
        let line = encode_line(payload);
        let mut out = line.as_bytes();
        while !out.is_empty() {
            match self.port.write(out) {
                Ok(0) => return Err(TxError::Io("串口写入 0 字节".into())),
                Ok(n) => out = &out[n..],
                Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
                Err(e) => return Err(TxError::Io(e.to_string())),
            }
        }
        let _ = self.port.flush();

        let deadline = Instant::now() + READ_TIMEOUT;
        loop {
            if let Some(pos) = self.buf.iter().position(|&b| b == b'\n') {
                let raw: Vec<u8> = self.buf.drain(..=pos).collect();
                let text = String::from_utf8_lossy(&raw);
                let text = text.trim_end_matches(['\r', '\n']);
                if text.is_empty() {
                    continue;
                }
                return parse_response(text).ok_or(TxError::BadLine);
            }
            if Instant::now() >= deadline {
                return Err(TxError::Timeout);
            }
            let mut chunk = [0u8; 256];
            match self.port.read(&mut chunk) {
                Ok(0) => {}
                Ok(n) => self.buf.extend_from_slice(&chunk[..n]),
                Err(e) if matches!(e.kind(), io::ErrorKind::TimedOut | io::ErrorKind::WouldBlock) => {}
                Err(e) => return Err(TxError::Io(e.to_string())),
            }
        }
    }
}

/// 枚举系统可用串口名。
pub fn list_ports() -> Vec<String> {
    serialport::available_ports()
        .unwrap_or_default()
        .into_iter()
        .map(|p| p.port_name)
        .collect()
}
