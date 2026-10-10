//! 虚拟 hub（演示模式，ADR-0004）：按协议 v1 应答的参考模拟器，遥测由模拟生成。
//!
//! 模拟行为：电压 5V 微波动；电流带噪声与缓慢漂移；4 个常驻虚拟设备
//! （真实 VID/PID）；每 20~40 秒随机插拔一个设备；`SW` 命令即时生效。
//! 放在协议 crate 内，使协议单测与演示模式都不依赖 tauri 运行时。

use std::time::{Duration, Instant};

use crate::{ChannelTelemetry, Response, CHANNELS};

/// 演示模式固件版本标识。
pub const DEMO_FW: &str = "0.9.0-demo";

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

    /// 每次被轮询时推进模拟：随机插拔事件 + 电流平滑演化。
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

    /// 当前模拟遥测。
    fn telemetry(&mut self) -> [ChannelTelemetry; CHANNELS] {
        std::array::from_fn(|i| {
            let c = &self.channels[i];
            if !c.on {
                return ChannelTelemetry { sw: false, mv: 0, c01ma: 0 };
            }
            let wave = 0.06 * c.phase.sin() + 1.0;
            let noise = 1.0 + (self.rng.next() - 0.5) * 0.05;
            let amps = (self.last_current[i] * wave * noise).max(0.0);
            let load_drop = (amps * 22.0) as u32; // 负载压降 mV
            let ripple = (self.rng.next() - 0.5) * 12.0;
            let mv = (5020.0 - load_drop as f64 + ripple).clamp(4900.0, 5120.0) as u32;
            ChannelTelemetry { sw: true, mv, c01ma: (amps * 10_000.0).round() as u32 }
        })
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
    /// 与 `SerialSession::transact` 同构：收到 payload，返回协议应答。
    pub fn transact(&mut self, payload: &str) -> Option<Response> {
        self.tick();
        let response = match payload {
            "PING" => Response::Pong,
            "PWR?" => Response::Pwr(self.telemetry()),
            "VER?" => Response::Ver { fw: DEMO_FW.into(), proto: 1, channels: CHANNELS as u8 },
            _ => {
                if let Some(rest) = payload.strip_prefix("SW,") {
                    let mut parts = rest.split(',');
                    let ch = parts.next().and_then(|s| s.parse::<u8>().ok());
                    let action = parts.next().unwrap_or("");
                    match (ch, action) {
                        (Some(ch), "ON") if (1..=CHANNELS as u8).contains(&ch) => {
                            self.channels[(ch - 1) as usize].on = true;
                            Response::Ok
                        }
                        (Some(ch), "OFF") if (1..=CHANNELS as u8).contains(&ch) => {
                            self.channels[(ch - 1) as usize].on = false;
                            Response::Ok
                        }
                        (Some(_), "ON" | "OFF") => {
                            Response::Err { code: 3, msg: "channel out of range".into() }
                        }
                        _ => Response::Err { code: 2, msg: "bad SW arguments".into() },
                    }
                } else {
                    Response::Err { code: 1, msg: "unknown command".into() }
                }
            }
        };
        Some(response)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn demo_responds_like_a_hub() {
        let mut hub = MockHub::new();
        assert_eq!(hub.transact("PING"), Some(Response::Pong));
        match hub.transact("PWR?") {
            Some(Response::Pwr(t)) => {
                assert_eq!(t.len(), 7);
                assert!(t[0].sw, "初始布景 CH1 有设备且上电");
                assert!(t[0].c01ma > 0);
            }
            other => panic!("期望 Pwr，实际 {other:?}"),
        }
        match hub.transact("VER?") {
            Some(Response::Ver { fw, proto, channels }) => {
                assert_eq!(fw, DEMO_FW);
                assert_eq!((proto, channels), (1, 7));
            }
            other => panic!("期望 Ver，实际 {other:?}"),
        }
    }

    #[test]
    fn demo_switch_takes_effect_immediately() {
        let mut hub = MockHub::new();
        assert_eq!(hub.transact("SW,1,OFF"), Some(Response::Ok));
        match hub.transact("PWR?") {
            Some(Response::Pwr(t)) => assert!(!t[0].sw && t[0].c01ma == 0),
            other => panic!("期望 Pwr，实际 {other:?}"),
        }
        assert_eq!(hub.transact("SW,1,ON"), Some(Response::Ok));
        assert_eq!(
            hub.transact("SW,9,ON"),
            Some(Response::Err { code: 3, msg: "channel out of range".into() })
        );
        assert_eq!(
            hub.transact("SW,x,ON"),
            Some(Response::Err { code: 2, msg: "bad SW arguments".into() })
        );
    }
}
