//! 串口协议 v1 编解码。规范见 `docs/protocol.md`。

pub mod mock;

use serde::Serialize;

pub const CHANNELS: usize = 7;
pub const MAX_LINE_LEN: usize = 128;

#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct ChannelTelemetry {
    pub sw: bool,
    /// 电压，单位 mV
    pub mv: u32,
    /// 电流，单位 0.1 mA
    pub c01ma: u32,
}

impl Default for ChannelTelemetry {
    fn default() -> Self {
        Self { sw: false, mv: 0, c01ma: 0 }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Response {
    Pong,
    Ver { fw: String, proto: u8, channels: u8 },
    Pwr([ChannelTelemetry; CHANNELS]),
    Ok,
    Err { code: u8, msg: String },
}

pub fn checksum(bytes: &[u8]) -> u8 {
    bytes.iter().fold(0u8, |acc, &b| acc ^ b)
}

/// 为 payload 附加 XOR 校验，生成完整行（不含行尾）。
pub fn encode_line(payload: &str) -> String {
    format!("{}*{:02X}", payload, checksum(payload.as_bytes()))
}

/// 校验一行并返回 payload；格式或校验不符返回 `None`。
pub fn decode_line(line: &str) -> Option<&str> {
    let line = line.trim_end_matches(['\r', '\n']);
    if line.is_empty() || line.len() > MAX_LINE_LEN {
        return None;
    }
    let (payload, sum) = line.rsplit_once('*')?;
    if sum.len() != 2 {
        return None;
    }
    let expected = u8::from_str_radix(sum, 16).ok()?;
    (checksum(payload.as_bytes()) == expected).then_some(payload)
}

/// 解析一条完整应答行（含校验）。
pub fn parse_response(line: &str) -> Option<Response> {
    let payload = decode_line(line)?;
    let mut fields = payload.split(',');

    match fields.next()? {
        "PONG" => (fields.next().is_none()).then_some(Response::Pong),
        "OK" => (fields.next().is_none()).then_some(Response::Ok),
        "VER" => {
            let fw = fields.next()?.to_string();
            let proto = fields.next()?.parse().ok()?;
            let channels = fields.next()?.parse().ok()?;
            (fields.next().is_none()).then_some(Response::Ver { fw, proto, channels })
        }
        "PWR" => {
            let mut telemetry = [ChannelTelemetry::default(); CHANNELS];
            for slot in &mut telemetry {
                let sw = fields.next()?;
                let mv = fields.next()?;
                let c01ma = fields.next()?;
                *slot = ChannelTelemetry {
                    sw: sw == "1",
                    mv: mv.parse().ok()?,
                    c01ma: c01ma.parse().ok()?,
                };
            }
            (fields.next().is_none()).then_some(Response::Pwr(telemetry))
        }
        "ERR" => {
            let code = fields.next()?.parse().ok()?;
            let msg = fields.next()?.to_string();
            (fields.next().is_none()).then_some(Response::Err { code, msg })
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn checksum_matches_spec_examples() {
        assert_eq!(encode_line("PING"), "PING*10");
        assert_eq!(encode_line("PONG"), "PONG*16");
        assert_eq!(encode_line("VER?"), "VER?*7E");
        assert_eq!(encode_line("PWR?"), "PWR?*6A");
        assert_eq!(encode_line("SW,3,OFF"), "SW,3,OFF*78");
        assert_eq!(encode_line("SW,3,ON"), "SW,3,ON*36");
        assert_eq!(encode_line("OK"), "OK*04");
    }

    #[test]
    fn decode_roundtrip() {
        assert_eq!(decode_line("PING*10"), Some("PING"));
        assert_eq!(decode_line("PING*10\r\n"), Some("PING"));
        // 小写校验和同样接受
        assert_eq!(decode_line("PING*10"), Some("PING"));
    }

    #[test]
    fn decode_rejects_bad_lines() {
        assert_eq!(decode_line("PING*11"), None, "校验不符");
        assert_eq!(decode_line("PING*1"), None, "校验长度不足");
        assert_eq!(decode_line("PING"), None, "缺少校验段");
        assert_eq!(decode_line("PING*ZZ"), None, "非十六进制");
        assert_eq!(decode_line(""), None, "空行");
    }

    #[test]
    fn parse_pwr_full_frame() {
        let payload = "PWR,1,5023,8421,0,0,0,1,5001,120,1,5019,33,0,0,0,1,4998,7,1,5002,88";
        let resp = parse_response(&encode_line(payload)).unwrap();
        match resp {
            Response::Pwr(t) => {
                assert!(t[0].sw && t[0].mv == 5023 && t[0].c01ma == 8421);
                assert!(!t[1].sw && t[1].mv == 0);
                assert!(t[6].sw && t[6].mv == 5002 && t[6].c01ma == 88);
            }
            other => panic!("期望 Pwr，实际 {other:?}"),
        }
    }

    #[test]
    fn parse_pwr_rejects_short_frame() {
        assert_eq!(parse_response(&encode_line("PWR,1,5023,8421")), None);
    }

    #[test]
    fn parse_misc_responses() {
        assert_eq!(parse_response(&encode_line("PONG")), Some(Response::Pong));
        assert_eq!(
            parse_response(&encode_line("VER,1.0.0,1,7")),
            Some(Response::Ver { fw: "1.0.0".into(), proto: 1, channels: 7 })
        );
        assert_eq!(parse_response(&encode_line("OK")), Some(Response::Ok));
        assert_eq!(
            parse_response(&encode_line("ERR,3,channel out of range")),
            Some(Response::Err { code: 3, msg: "channel out of range".into() })
        );
        assert_eq!(parse_response(&encode_line("WHAT,1")), None);
    }
}
