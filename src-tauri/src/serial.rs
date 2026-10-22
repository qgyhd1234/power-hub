//! 串口会话：打开、收发事务（二进制帧协议，一问一答）。

use std::io;
use std::time::{Duration, Instant};

use log::debug;

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
        // 原始帧收发十六进制 dump（Debug 级，spec `.scratch/power-hub-logs/`）
        debug!("串口 → {:02X?}", frame);
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
            // Debug 级下快照接收缓冲：除重建帧外，把失步被丢弃的字节也暴露给联调
            let raw = if log::log_enabled!(log::Level::Debug) {
                Some(self.buf.clone())
            } else {
                None
            };
            if let Some(resp) = extract_response(&mut self.buf) {
                debug!("串口 ← {:02X?}", resp.encode());
                if let Some(raw) = raw {
                    debug!("串口 ← 线上缓冲（含失步丢弃字节）: {:02X?}", raw);
                }
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

/// 串口条目：名称 + USB 身份（蓝牙等非 USB 口 vid/pid 为 None）。
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PortInfo {
    pub name: String,
    pub vid: Option<u16>,
    pub pid: Option<u16>,
}

/// 串口列表结果：`filtered = false` 表示 CH340 过滤无命中，已回退展示全部。
#[derive(Debug, PartialEq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PortList {
    pub ports: Vec<PortInfo>,
    pub filtered: bool,
}

/// 枚举系统串口并按 CH340 魔术字（`magic.rs`）过滤；无命中回退展示全部。
pub fn list_ports() -> PortList {
    filter_control_port(enumerate())
}

fn enumerate() -> Vec<PortInfo> {
    serialport::available_ports()
        .unwrap_or_default()
        .into_iter()
        // 蓝牙虚拟串口不可能是 hub 控制口，且打开有阻塞风险，始终排除
        .filter(|p| !matches!(p.port_type, serialport::SerialPortType::BluetoothPort))
        .map(|p| {
            let (vid, pid) = match p.port_type {
                serialport::SerialPortType::UsbPort(info) => (Some(info.vid), Some(info.pid)),
                _ => (None, None),
            };
            PortInfo { name: p.port_name, vid, pid }
        })
        .collect()
}

/// 过滤出 CH340 控制口；无命中则原样返回全部（回退策略）。
fn filter_control_port(all: Vec<PortInfo>) -> PortList {
    let is_control = |p: &PortInfo| {
        p.vid == Some(crate::magic::CONTROL_VID) && p.pid == Some(crate::magic::CONTROL_PID)
    };
    if all.iter().any(is_control) {
        PortList { ports: all.into_iter().filter(is_control).collect(), filtered: true }
    } else {
        PortList { ports: all, filtered: false }
    }
}

#[cfg(test)]
mod port_filter_tests {
    use super::*;

    fn port(name: &str, vid: Option<u16>, pid: Option<u16>) -> PortInfo {
        PortInfo { name: name.into(), vid, pid }
    }

    #[test]
    fn keeps_only_ch340_when_present() {
        let all = vec![
            port("COM37", Some(0x1A86), Some(0x7523)),
            port("COM35", Some(0x1A86), Some(0x55D4)), // CH9102，同为 WCH 但不是 CH340
            port("COM31", Some(0x1A86), Some(0x8010)), // WCH-Link
            port("COM25", Some(0x0D28), Some(0x0202)),
        ];
        let list = filter_control_port(all);
        assert!(list.filtered);
        assert_eq!(list.ports, vec![port("COM37", Some(0x1A86), Some(0x7523))]);
    }

    #[test]
    fn falls_back_to_all_when_no_ch340() {
        let all = vec![
            port("COM35", Some(0x1A86), Some(0x55D4)),
            port("COM25", Some(0x0D28), Some(0x0202)),
            port("COM7", None, None),
        ];
        let list = filter_control_port(all);
        assert!(!list.filtered);
        assert_eq!(list.ports.len(), 3);
    }

    #[test]
    fn empty_list_stays_empty_and_unfiltered() {
        let list = filter_control_port(vec![]);
        assert!(!list.filtered);
        assert!(list.ports.is_empty());
    }
}

// 写帧依赖 Command::encode 的定长布局，这里把规范帧长锁进编译期。
const _: () = assert!(crate::protocol::CMD_FRAME_LEN == 4, "上位机→hub 帧长按规范为 4 字节");
