//! 虚拟 hub（演示模式，ADR-0004）：按二进制帧协议应答的参考实现，遥测由模拟生成。
//!
//! 模拟行为：电流带噪声与缓慢漂移；4 个常驻虚拟设备（真实 VID/PID）；
//! 每 20~40 秒随机插拔一个设备；通道开关即时生效。
//! 放在协议 crate 内，使协议单测与演示模式都不依赖 tauri 运行时。

use std::time::{Duration, Instant};

use crate::{parse_request, ChannelTelemetry, Request, Response, CHANNELS, PROTO_VERSION, RESP_FRAME_LEN};

/// 演示模式固件版本字节（协议 §4.1：fw 单字节，上位机仅透传显示）。
pub const DEMO_FW: u8 = 0x00;

/// 模拟端口上的虚拟设备信息（演示模式下供上位机展示）。
#[derive(Debug, Clone, PartialEq)]
pub struct DemoDevice {
    pub name: &'static str,
    pub vid: u16,
    pub pid: u16,
}

struct MockDevice {
    name: &'static str,
    vid: u16,
    pid: u16,
    /// 额定电流（A）
    current: f64,
}

const DEVICE_LIBRARY: [MockDevice; 5] = [
    MockDevice { name: "iPhone 15", vid: 0x05AC, pid: 0x12A8, current: 0.48 },
    MockDevice { name: "SanDisk Ultra U 盘", vid: 0x0781, pid: 0x5583, current: 0.06 },
    MockDevice { name: "USB 接收器 (Logitech)", vid: 0x046D, pid: 0xC52B, current: 0.02 },
    MockDevice { name: "机械键盘", vid: 0x0C45, pid: 0x652F, current: 0.12 },
    MockDevice { name: "Seagate 移动硬盘", vid: 0x0BC2, pid: 0x2344, current: 0.25 },
];

struct MockChannel {
    on: bool,
    device: Option<usize>, // DEVICE_LIBRARY 下标
    phase: f64,
}

struct Rng(u64);

impl Rng {
    fn new() -> Self {
        let seed = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos() as u64)
            .unwrap_or(0x9E3779B97F4A7C15);
        Rng(seed | 1)
    }

    /// [0, 1) 均匀分布（xorshift64）
    fn next(&mut self) -> f64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        (x >> 11) as f64 / (1u64 << 53) as f64
    }

    fn range(&mut self, lo: f64, hi: f64) -> f64 {
        lo + self.next() * (hi - lo)
    }
}

pub struct MockHub {
    channels: [MockChannel; CHANNELS],
    /// 各通道上一次输出电流（A），用于平滑
    last_current: [f64; CHANNELS],
    next_event: Instant,
    rng: Rng,
}

impl MockHub {
    pub fn new() -> Self {
        // 初始布景：CH1/2/4/5 有设备，CH3/6/7 空
        let initial: [Option<usize>; CHANNELS] = [Some(0), Some(1), None, Some(3), Some(4), None, None];
        let mut hub = Self {
            channels: std::array::from_fn(|i| MockChannel {
                on: true,
                device: initial[i],
                phase: i as f64 * 1.3,
            }),
            last_current: [0.0; CHANNELS],
            next_event: Instant::now() + Duration::from_secs(20),
            rng: Rng::new(),
        };
        for i in 0..CHANNELS {
            hub.last_current[i] = hub.base_current(i);
        }
        hub
    }

    fn base_current(&self, ch: usize) -> f64 {
        let c = &self.channels[ch];
        match (c.on, c.device) {
            (true, Some(d)) => DEVICE_LIBRARY[d].current,
            (true, None) => 0.002, // 空载微漏流
            (false, _) => 0.0,
        }
    }

    /// 每次被请求时推进模拟：随机插拔事件 + 电流平滑演化。
    fn tick(&mut self) {
        if Instant::now() >= self.next_event {
            self.random_plug_event();
            self.next_event = Instant::now() + Duration::from_secs(self.rng.range(20.0, 40.0) as u64);
        }
        for i in 0..CHANNELS {
            self.channels[i].phase += 0.15;
            // 向基准电流缓慢靠拢，制造自然过渡
            let target = self.base_current(i);
            self.last_current[i] += (target - self.last_current[i]) * 0.35;
        }
    }

    fn random_plug_event(&mut self) {
        let empty: Vec<usize> = (0..CHANNELS).filter(|&i| self.channels[i].device.is_none()).collect();
        let occupied: Vec<usize> = (0..CHANNELS).filter(|&i| self.channels[i].device.is_some()).collect();
        // 空口多倾向插入，设备多倾向拔出，维持 2~5 个在线
        let plug = if occupied.len() <= 2 {
            true
        } else if occupied.len() >= 5 || empty.is_empty() {
            false
        } else {
            self.rng.next() < 0.5
        };
        if plug {
            if let Some(&ch) = empty.get((self.rng.next() * empty.len() as f64) as usize) {
                let device = (self.rng.next() * DEVICE_LIBRARY.len() as f64) as usize;
                self.channels[ch].device = Some(device.min(DEVICE_LIBRARY.len() - 1));
            }
        } else if let Some(&ch) = occupied.get((self.rng.next() * occupied.len() as f64) as usize) {
            self.channels[ch].device = None;
        }
    }

    /// 7 通道电源状态掩码（bit0 = 通道 1）。
    fn switches(&self) -> u8 {
        self.channels.iter().enumerate().fold(
            0u8,
            |acc, (i, c)| if c.on { acc | (1 << i) } else { acc },
        )
    }

    /// 当前通道模拟电流（mA）。
    fn current_ma(&mut self, ch: usize) -> f32 {
        let c = &self.channels[ch];
        if !c.on {
            return 0.0;
        }
        let wave = 0.06 * c.phase.sin() + 1.0;
        let noise = 1.0 + (self.rng.next() - 0.5) * 0.05;
        ((self.last_current[ch] * wave * noise).max(0.0) * 1000.0) as f32
    }

    /// 当前模拟遥测（开关态 + 电流；电压不上传）。
    pub fn telemetry(&mut self) -> [ChannelTelemetry; CHANNELS] {
        std::array::from_fn(|i| ChannelTelemetry { sw: self.channels[i].on, ma: self.current_ma(i) })
    }

    /// 供上位机注入共享状态的设备表。
    pub fn device_infos(&self) -> [Option<DemoDevice>; CHANNELS] {
        std::array::from_fn(|i| {
            self.channels[i].device.map(|d| DemoDevice {
                name: DEVICE_LIBRARY[d].name,
                vid: DEVICE_LIBRARY[d].vid,
                pid: DEVICE_LIBRARY[d].pid,
            })
        })
    }
}

impl MockHub {
    /// 处理一帧 4 字节请求，返回 7 字节应答帧。
    /// 请求非法（静默丢弃语义，规范 §6）返回 `None`，不应答。
    pub fn handle(&mut self, request_frame: &[u8]) -> Option<[u8; RESP_FRAME_LEN]> {
        let request = parse_request(request_frame)?;
        self.tick();
        let response = match request {
            Request::Handshake => Response::Handshake {
                fw: DEMO_FW,
                proto: PROTO_VERSION,
                channels: CHANNELS as u8,
                switches: self.switches(),
            },
            Request::SetChannel { channel, on } => {
                self.channels[(channel - 1) as usize].on = on;
                Response::Switches(self.switches())
            }
            Request::ReadCurrent { channel } => Response::Current {
                channel,
                ma: self.current_ma((channel - 1) as usize),
            },
        };
        Some(response.encode())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{cmd, decode_response, Command, CMD_FRAME_LEN};

    /// 环回辅助：发原始请求帧，解出应答。
    fn ask(hub: &mut MockHub, request: &[u8]) -> Response {
        decode_response(&hub.handle(request).expect("合法请求必有应答")).expect("应答可解码")
    }

    fn cmd_frame(command: &Command) -> [u8; CMD_FRAME_LEN] {
        command.encode()
    }

    #[test]
    fn handshake_identifies_as_hub() {
        let mut hub = MockHub::new();
        let resp = ask(&mut hub, &cmd_frame(&Command::Handshake));
        assert!(resp.is_valid_handshake(), "演示 hub 必须通过识别判据");
        match resp {
            Response::Handshake { fw, switches, .. } => {
                assert_eq!(fw, DEMO_FW);
                assert_eq!(switches, 0x7F, "初始 7 通道全开");
            }
            other => panic!("期望 Handshake，实际 {other:?}"),
        }
    }

    #[test]
    fn full_flow_handshake_set_read() {
        let mut hub = MockHub::new();
        // 握手 → 关通道 1 → 读通道 1 电流为 0，读通道 2 有设备电流 > 0
        assert!(ask(&mut hub, &cmd_frame(&Command::Handshake)).is_valid_handshake());
        assert_eq!(
            ask(&mut hub, &cmd_frame(&Command::set_channel(1, false).unwrap())),
            Response::Switches(0x7E)
        );
        assert_eq!(
            ask(&mut hub, &cmd_frame(&Command::read_current(1).unwrap())),
            Response::Current { channel: 1, ma: 0.0 }
        );
        match ask(&mut hub, &cmd_frame(&Command::read_current(2).unwrap())) {
            Response::Current { channel: 2, ma } => assert!(ma > 0.0),
            other => panic!("期望 Current，实际 {other:?}"),
        }
    }

    #[test]
    fn invalid_requests_are_silently_dropped() {
        let mut hub = MockHub::new();
        assert_eq!(hub.handle(&[]), None, "空帧");
        assert_eq!(hub.handle(&[0xA5, 0x00, 0x00]), None, "长度不足");
        assert_eq!(hub.handle(&[0xA5, 0x00, 0x00, 0x00]), None, "包尾不对");
        assert_eq!(hub.handle(&[0xA5, 0x03, 0x00, 0x5A]), None, "未知指令");
        assert_eq!(hub.handle(&[0xA5, 0x01, 0x00, 0x5A]), None, "设置通道 0");
        assert_eq!(hub.handle(&[0xA5, 0x02, 0x08, 0x5A]), None, "读通道 8");
    }

    #[test]
    fn response_frames_have_framing_bytes() {
        let mut hub = MockHub::new();
        let frame = hub.handle(&cmd_frame(&Command::Handshake)).unwrap();
        assert_eq!(frame[0], crate::FRAME_HEAD);
        assert_eq!(frame[6], crate::FRAME_TAIL);
        assert_eq!(frame[1], cmd::HANDSHAKE);
    }
}
