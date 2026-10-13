//! 串口二进制帧协议编解码（协议版本 proto=1）。规范见 `docs/protocol.md`。

pub mod mock;

use serde::Serialize;

/// 通道数，协议锁定为 7。
pub const CHANNELS: usize = 7;
/// 上位机 → hub 帧长：包头 + 指令 + 数据 + 包尾。
pub const CMD_FRAME_LEN: usize = 4;
/// hub → 上位机 帧长：包头 + 指令 + 4B 数据 + 包尾。
pub const RESP_FRAME_LEN: usize = 7;
/// 包头。
pub const FRAME_HEAD: u8 = 0xA5;
/// 包尾。
pub const FRAME_TAIL: u8 = 0x5A;
/// 协议版本（握手应答 proto 字节）。
pub const PROTO_VERSION: u8 = 0x01;

/// 指令字节（`docs/protocol.md` §4）。
pub mod cmd {
    /// 握手（版本查询）
    pub const HANDSHAKE: u8 = 0x00;
    /// 通道设置
    pub const SET_CHANNEL: u8 = 0x01;
    /// 读通道电流
    pub const READ_CURRENT: u8 = 0x02;
    /// 电流应答指令基值：应答 = `CURRENT_RESP_BASE | 通道号`
    pub const CURRENT_RESP_BASE: u8 = 0xF0;
}

/// 单通道遥测：开关态与电流（规范 §5：电压不上传，恒按 5V 假定）。
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct ChannelTelemetry {
    pub sw: bool,
    /// 电流，单位 mA
    pub ma: f32,
}

impl Default for ChannelTelemetry {
    fn default() -> Self {
        Self { sw: false, ma: 0.0 }
    }
}

/// 上位机 → hub 命令（编码为 4 字节定长帧）。
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Command {
    Handshake,
    SetChannel { channel: u8, on: bool },
    ReadCurrent { channel: u8 },
}

impl Command {
    /// 通道号合法性（1~7）在构造时校验。
    pub fn set_channel(channel: u8, on: bool) -> Option<Self> {
        is_channel(channel).then_some(Self::SetChannel { channel, on })
    }

    pub fn read_current(channel: u8) -> Option<Self> {
        is_channel(channel).then_some(Self::ReadCurrent { channel })
    }

    pub fn encode(&self) -> [u8; CMD_FRAME_LEN] {
        let (cmd_byte, data) = match *self {
            Self::Handshake => (cmd::HANDSHAKE, 0x00),
            Self::SetChannel { channel, on } => {
                (cmd::SET_CHANNEL, channel | if on { 0x10 } else { 0x00 })
            }
            Self::ReadCurrent { channel } => (cmd::READ_CURRENT, channel),
        };
        [FRAME_HEAD, cmd_byte, data, FRAME_TAIL]
    }
}

/// hub 侧解析出的一帧请求。
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Request {
    Handshake,
    SetChannel { channel: u8, on: bool },
    ReadCurrent { channel: u8 },
}

/// 解析一帧 4 字节请求；包头/包尾/指令/参数任一非法返回 `None`（静默丢弃语义）。
pub fn parse_request(frame: &[u8]) -> Option<Request> {
    if frame.len() != CMD_FRAME_LEN || frame[0] != FRAME_HEAD || frame[3] != FRAME_TAIL {
        return None;
    }
    let data = frame[2];
    match frame[1] {
        cmd::HANDSHAKE => Some(Request::Handshake), // 数据字节固件忽略
        cmd::SET_CHANNEL => {
            let on = data & 0x10 != 0;
            let channel = data & 0x0F;
            (data & 0xE0 == 0 && is_channel(channel))
                .then_some(Request::SetChannel { channel, on })
        }
        cmd::READ_CURRENT => (data & 0xF8 == 0 && is_channel(data))
            .then_some(Request::ReadCurrent { channel: data }),
        _ => None,
    }
}

/// hub → 上位机 应答（编码为 7 字节定长帧）。
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Response {
    /// 0x00 握手应答：fw / proto / 通道数 / 通道状态掩码
    Handshake {
        fw: u8,
        proto: u8,
        channels: u8,
        switches: u8,
    },
    /// 0x01 应答：动作执行后的 7 通道状态掩码
    Switches(u8),
    /// 0xF0|通道号：通道电流（mA）
    Current { channel: u8, ma: f32 },
}

impl Response {
    pub fn encode(&self) -> [u8; RESP_FRAME_LEN] {
        let mut frame = [0x00; RESP_FRAME_LEN];
        frame[0] = FRAME_HEAD;
        frame[RESP_FRAME_LEN - 1] = FRAME_TAIL;
        match *self {
            Self::Handshake { fw, proto, channels, switches } => {
                frame[1] = cmd::HANDSHAKE;
                frame[2] = fw;
                frame[3] = proto;
                frame[4] = channels;
                frame[5] = switches & 0x7F;
            }
            Self::Switches(switches) => {
                frame[1] = cmd::SET_CHANNEL;
                frame[2] = switches & 0x7F;
            }
            Self::Current { channel, ma } => {
                frame[1] = cmd::CURRENT_RESP_BASE | channel;
                frame[2..6].copy_from_slice(&ma.to_le_bytes());
            }
        }
        frame
    }

    /// 握手识别判据（规范 §4.1）：proto = 1 且通道数 = 7；fw 与状态不参与。
    pub fn is_valid_handshake(&self) -> bool {
        match self {
            Self::Handshake { proto, channels, .. } => {
                *proto == PROTO_VERSION && *channels == CHANNELS as u8
            }
            _ => false,
        }
    }
}

/// 解码一帧 7 字节应答；结构非法返回 `None`。
pub fn decode_response(frame: &[u8]) -> Option<Response> {
    if frame.len() != RESP_FRAME_LEN || frame[0] != FRAME_HEAD || frame[RESP_FRAME_LEN - 1] != FRAME_TAIL
    {
        return None;
    }
    let data: [u8; 4] = frame[2..6].try_into().ok()?;
    match frame[1] {
        cmd::HANDSHAKE => Some(Response::Handshake {
            fw: data[0],
            proto: data[1],
            channels: data[2],
            switches: data[3] & 0x7F,
        }),
        // 状态掩码 bit7 恒 0，否则视为垃圾帧
        cmd::SET_CHANNEL => (data[0] & 0x80 == 0).then_some(Response::Switches(data[0] & 0x7F)),
        c if c & 0xF0 == cmd::CURRENT_RESP_BASE => {
            let channel = c & 0x0F;
            is_channel(channel).then_some(Response::Current {
                channel,
                ma: f32::from_le_bytes(data),
            })
        }
        _ => None,
    }
}

/// 从接收缓冲中提取一帧应答（失步重同步，规范 §3）。
///
/// 无包头时清空缓冲返回 `None`；帧不完整时保留已收字节等待更多数据；
/// 包头后定长字节非法（伪包头/坏帧）时仅丢弃该包头字节继续搜索。
pub fn extract_response(buf: &mut Vec<u8>) -> Option<Response> {
    loop {
        let Some(head) = buf.iter().position(|&b| b == FRAME_HEAD) else {
            buf.clear();
            return None;
        };
        buf.drain(..head);
        if buf.len() < RESP_FRAME_LEN {
            return None;
        }
        match decode_response(&buf[..RESP_FRAME_LEN]) {
            Some(resp) => {
                buf.drain(..RESP_FRAME_LEN);
                return Some(resp);
            }
            None => {
                buf.remove(0);
            }
        }
    }
}

fn is_channel(ch: u8) -> bool {
    (1..=CHANNELS as u8).contains(&ch)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encode_matches_spec_examples() {
        assert_eq!(Command::Handshake.encode(), [0xA5, 0x00, 0x00, 0x5A]);
        assert_eq!(Command::set_channel(2, true).unwrap().encode(), [0xA5, 0x01, 0x12, 0x5A]);
        assert_eq!(Command::set_channel(2, false).unwrap().encode(), [0xA5, 0x01, 0x02, 0x5A]);
        assert_eq!(Command::read_current(3).unwrap().encode(), [0xA5, 0x02, 0x03, 0x5A]);
    }

    #[test]
    fn command_rejects_invalid_channels() {
        assert_eq!(Command::set_channel(0, true), None);
        assert_eq!(Command::set_channel(8, true), None);
        assert_eq!(Command::read_current(0), None);
        assert_eq!(Command::read_current(8), None);
    }

    #[test]
    fn current_roundtrip_spec_example() {
        // 规范 §4.3 示例帧：通道 3 → A5 F3 84 01 80 44 5A
        let ma = f32::from_bits(0x4480_0184);
        let resp = Response::Current { channel: 3, ma };
        assert_eq!(resp.encode(), [0xA5, 0xF3, 0x84, 0x01, 0x80, 0x44, 0x5A]);
        assert_eq!(decode_response(&resp.encode()), Some(resp));
    }

    #[test]
    fn handshake_and_switches_encode() {
        let hs = Response::Handshake { fw: 0x00, proto: 0x01, channels: 0x07, switches: 0x7F };
        assert_eq!(hs.encode(), [0xA5, 0x00, 0x00, 0x01, 0x07, 0x7F, 0x5A]);
        assert_eq!(
            Response::Switches(0x7E).encode(),
            [0xA5, 0x01, 0x7E, 0x00, 0x00, 0x00, 0x5A]
        );
    }

    #[test]
    fn handshake_criteria() {
        // fw 与状态不参与判据
        let ok = Response::Handshake { fw: 0xAB, proto: 0x01, channels: 0x07, switches: 0x00 };
        assert!(ok.is_valid_handshake());
        let bad_proto = Response::Handshake { fw: 0, proto: 0x02, channels: 0x07, switches: 0 };
        assert!(!bad_proto.is_valid_handshake());
        let bad_channels = Response::Handshake { fw: 0, proto: 0x01, channels: 0x05, switches: 0 };
        assert!(!bad_channels.is_valid_handshake());
        assert!(!Response::Switches(0x7F).is_valid_handshake());
    }

    #[test]
    fn decode_rejects_invalid_frames() {
        let good = Response::Switches(0x7F).encode();
        let mut bad_tail = good;
        bad_tail[6] = 0x00;
        assert_eq!(decode_response(&bad_tail), None, "包尾不对");
        let mut bad_head = good;
        bad_head[0] = 0x00;
        assert_eq!(decode_response(&bad_head), None, "包头不对");
        let mut bad_cmd = good;
        bad_cmd[1] = 0x03;
        assert_eq!(decode_response(&bad_cmd), None, "未知指令");
        let mut ch0 = Response::Current { channel: 1, ma: 1.0 }.encode();
        ch0[1] = 0xF0;
        assert_eq!(decode_response(&ch0), None, "电流应答通道 0");
        let mut ch8 = ch0;
        ch8[1] = 0xF8;
        assert_eq!(decode_response(&ch8), None, "电流应答通道 8");
        let mut hi_bit = Response::Switches(0x7F).encode();
        hi_bit[2] = 0x80;
        assert_eq!(decode_response(&hi_bit), None, "状态掩码 bit7 恒 0");
        assert_eq!(decode_response(&good[..6]), None, "长度不足");
    }

    #[test]
    fn extract_recovers_from_garbage_prefix() {
        let frame = Response::Current { channel: 3, ma: f32::from_bits(0x4480_0184) }.encode();
        let mut buf = vec![0x00, 0xFF, 0x13];
        buf.extend_from_slice(&frame);
        assert_eq!(extract_response(&mut buf), decode_response(&frame));
        assert!(buf.is_empty());
    }

    #[test]
    fn extract_waits_for_incomplete_frame() {
        let frame = Response::Switches(0x55).encode();
        let mut buf = frame[..5].to_vec();
        assert_eq!(extract_response(&mut buf), None);
        assert_eq!(buf, frame[..5], "不完整帧保留待续");
        buf.extend_from_slice(&frame[5..]);
        assert_eq!(extract_response(&mut buf), Some(Response::Switches(0x55)));
    }

    #[test]
    fn extract_skips_fake_head() {
        let good = Response::Switches(0x33).encode();
        let mut buf = vec![0xA5, 0xFF, 0x01, 0x02, 0x03, 0x04, 0x05];
        buf.extend_from_slice(&good);
        assert_eq!(extract_response(&mut buf), Some(Response::Switches(0x33)));
        assert!(buf.is_empty());
    }

    #[test]
    fn extract_clears_headless_garbage() {
        let mut buf = vec![0x01, 0x02, 0x03];
        assert_eq!(extract_response(&mut buf), None);
        assert!(buf.is_empty());
    }

    #[test]
    fn extract_two_frames_in_sequence() {
        let mut buf = Response::Switches(0x01).encode().into_iter().collect::<Vec<u8>>();
        buf.extend(Response::Switches(0x02).encode());
        assert_eq!(extract_response(&mut buf), Some(Response::Switches(0x01)));
        assert_eq!(extract_response(&mut buf), Some(Response::Switches(0x02)));
    }

    #[test]
    fn parse_request_accepts_valid_frames() {
        assert_eq!(parse_request(&[0xA5, 0x00, 0x00, 0x5A]), Some(Request::Handshake));
        // 握手数据字节固件忽略
        assert_eq!(parse_request(&[0xA5, 0x00, 0x42, 0x5A]), Some(Request::Handshake));
        assert_eq!(
            parse_request(&[0xA5, 0x01, 0x12, 0x5A]),
            Some(Request::SetChannel { channel: 2, on: true })
        );
        assert_eq!(
            parse_request(&[0xA5, 0x01, 0x02, 0x5A]),
            Some(Request::SetChannel { channel: 2, on: false })
        );
        assert_eq!(
            parse_request(&[0xA5, 0x02, 0x07, 0x5A]),
            Some(Request::ReadCurrent { channel: 7 })
        );
    }

    #[test]
    fn parse_request_rejects_invalid() {
        assert_eq!(parse_request(&[0xA5, 0x00, 0x00]), None, "长度不足");
        assert_eq!(parse_request(&[0x5A, 0x00, 0x00, 0x5A]), None, "包头不对");
        assert_eq!(parse_request(&[0xA5, 0x00, 0x00, 0x00]), None, "包尾不对");
        assert_eq!(parse_request(&[0xA5, 0x03, 0x00, 0x5A]), None, "未知指令");
        assert_eq!(parse_request(&[0xA5, 0x01, 0x00, 0x5A]), None, "设置通道 0");
        assert_eq!(parse_request(&[0xA5, 0x01, 0x08, 0x5A]), None, "设置通道 8");
        assert_eq!(parse_request(&[0xA5, 0x01, 0x20, 0x5A]), None, "bit5 必须为 0");
        assert_eq!(parse_request(&[0xA5, 0x02, 0x00, 0x5A]), None, "读通道 0");
        assert_eq!(parse_request(&[0xA5, 0x02, 0x08, 0x5A]), None, "读通道 8");
    }
}
