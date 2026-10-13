//! 串口会话：打开、收发事务（二进制帧协议，一问一答）。

use std::io;
use std::time::{Duration, Instant};

use crate::protocol::{extract_response, Command, Response};

pub const BAUD: u32 = 115_200;
pub const READ_TIMEOUT: Duration = Duration::from_millis(200);

#[derive(Debug, PartialEq)]
pub enum TxError {
    Timeout,
    Io(String),
}

impl std::fmt::Display for TxError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TxError::Timeout => write!(f, "hub 应答超时"),
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

    /// 发送一条命令并等待应答帧（`docs/protocol.md` §2 一问一答）。
    ///
    /// 严格一问一答下不存在对端主动数据，事务开始时丢弃上次遗留的
    /// 未完成字节，避免上一条超时命令的迟到应答污染本次解析。
    /// 非法帧由解析层静默丢弃，这里只会以超时告终。
    pub fn transact(&mut self, command: &Command) -> Result<Response, TxError> {
        self.buf.clear();
        let frame = command.encode();
        let mut out: &[u8] = &frame;
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
            if let Some(resp) = extract_response(&mut self.buf) {
                return Ok(resp);
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

// 写帧依赖 Command::encode 的定长布局，这里把规范帧长锁进编译期。
const _: () = assert!(crate::protocol::CMD_FRAME_LEN == 4, "上位机→hub 帧长按规范为 4 字节");
