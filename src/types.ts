/** 与 Rust 侧 serde camelCase 序列化对齐（src-tauri/src/state.rs）。 */

export interface DeviceInfo {
  name: string;
  vid: number;
  pid: number;
}

/** 串口条目：蓝牙等非 USB 口 vid/pid 为 null */
export interface PortInfo {
  name: string;
  vid: number | null;
  pid: number | null;
}

/** 串口列表结果：filtered=false 表示 CH340 过滤无命中，已回退展示全部 */
export interface PortList {
  ports: PortInfo[];
  filtered: boolean;
}

export interface ChannelStatus {
  index: number;
  on: boolean;
  /** V */
  voltage: number;
  /** A */
  current: number;
  /** W */
  power: number;
  /** 电流历史（A），最旧在前 */
  history: number[];
  device: DeviceInfo | null;
}

export interface HubStatus {
  connected: boolean;
  /** 演示模式（ADR-0004）：数据来自内置模拟器 */
  demo: boolean;
  port: string | null;
  fwVersion: string | null;
  pollIntervalMs: number;
  /** 状态轮询间隔（毫秒）；0 = 禁用状态轮询 */
  statusPollIntervalMs: number;
  channels: ChannelStatus[];
  totalPower: number;
  totalCurrent: number;
  deviceCount: number;
}

export function fmtVoltage(v: number): string {
  return `${v.toFixed(2)} V`;
}

export function fmtCurrent(a: number): string {
  if (Math.abs(a) < 0.1) return `${(a * 1000).toFixed(1)} mA`;
  return `${a.toFixed(3)} A`;
}

export function fmtPower(w: number): string {
  if (Math.abs(w) < 1) return `${(w * 1000).toFixed(0)} mW`;
  return `${w.toFixed(2)} W`;
}

export function fmtVidPid(d: DeviceInfo): string {
  const hex = (n: number) => n.toString(16).toUpperCase().padStart(4, "0");
  return `${hex(d.vid)}:${hex(d.pid)}`;
}
