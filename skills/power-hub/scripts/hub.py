#!/usr/bin/env python
"""power-hub 控制：7 通道遥测查询与电源控制。

连接方式自动选择：
1. 上位机本地 API（http://127.0.0.1:8756，需要上位机在运行且已连接 hub）
2. 直连串口（pyserial，按 CH340 魔术字过滤优先控制口，无命中回退全口
   握手识别，二进制帧协议）

用法（协议规范见仓库 docs/protocol.md）：
    uv run --with pyserial scripts/hub.py <子命令> [参数]
"""

from __future__ import annotations

import argparse
import json
import struct
import sys
import time
import urllib.error
import urllib.request
from typing import NamedTuple

API_BASE = "http://127.0.0.1:8756"
API_HEADERS = {"X-Power-Hub": "v1", "Accept": "application/json"}
BAUD = 115200
CHANNELS = 7

# CH340 控制口魔术字（权威定义：src-tauri/src/magic.rs、docs/protocol.md §9）。
# vid/pid 精确匹配：CH9102（1A86:55D4）、WCH-Link（1A86:8010）等同厂芯片不放行。
CONTROL_VID = 0x1A86
CONTROL_PID = 0x7523

# ---------- 二进制帧协议编解码（权威规范：docs/protocol.md） ----------

FRAME_HEAD = 0xA5
FRAME_TAIL = 0x5A
CMD_LEN = 4   # 上位机 → hub：0xA5 + 指令(1B) + 数据(1B) + 0x5A
RESP_LEN = 7  # hub → 上位机：0xA5 + 指令(1B) + 数据(4B) + 0x5A
RESPONSE_TIMEOUT = 0.2  # 应答超时 200 ms

CMD_HANDSHAKE = 0x00      # 握手（版本查询）
CMD_CHANNEL = 0x01        # 通道设置
CMD_CURRENT = 0x02        # 读通道电流
CURRENT_RESP_BASE = 0xF0  # 读电流应答指令 = 0xF0 | 通道号（0xF1~0xF7）

VALID_RESP_CMDS = {CMD_HANDSHAKE, CMD_CHANNEL} | {
    CURRENT_RESP_BASE | ch for ch in range(1, CHANNELS + 1)
}


class HubError(Exception):
    pass


def encode_cmd(cmd: int, data: int) -> bytes:
    """4 字节命令帧。"""
    return bytes((FRAME_HEAD, cmd, data, FRAME_TAIL))


def extract_frame(buf: bytearray) -> tuple[int, bytes] | None:
    """从缓冲提取一帧 7 字节应答：定位 0xA5 后取定长，校验包尾与指令白名单；
    不合法则整帧丢弃、从下一字节重新同步。凑不满一帧返回 None。

    返回 (指令, 4 字节数据)。
    """
    while True:
        i = buf.find(FRAME_HEAD)
        if i < 0:
            del buf[:]
            return None
        if i > 0:
            del buf[:i]
        if len(buf) < RESP_LEN:
            return None
        frame = bytes(buf[:RESP_LEN])
        if frame[RESP_LEN - 1] == FRAME_TAIL and frame[1] in VALID_RESP_CMDS:
            del buf[:RESP_LEN]
            return frame[1], frame[2 : RESP_LEN - 1]
        del buf[:1]  # 非法帧：从下一字节重新同步


# ---------- 传输层 ----------

class ApiTransport:
    """经上位机本地 API（推荐：数据同源、含设备信息、不占串口）。"""

    def __init__(self) -> None:
        self.info = "本地 API (127.0.0.1:8756)"

    def _request(self, path: str, method: str = "GET", timeout: float = 2.0) -> bytes:
        req = urllib.request.Request(API_BASE + path, headers=API_HEADERS, method=method)
        with urllib.request.urlopen(req, timeout=timeout) as resp:
            return resp.read()

    def status(self) -> dict:
        return json.loads(self._request("/api/status").decode("utf-8"))

    def set_channel(self, ch: int, on: bool) -> None:
        action = "on" if on else "off"
        try:
            self._request(f"/api/channel/{ch}/{action}", method="POST", timeout=3.0)
        except urllib.error.HTTPError as e:
            raise HubError(f"API 返回 {e.code}: {e.read().decode('utf-8', 'replace')}") from None


class SerialTransport:
    """直连串口（上位机未运行时的降级路径；无设备信息）。"""

    def __init__(self, port: str) -> None:
        import serial  # 提供：uv run --with pyserial

        self.port_name = port
        self.ser = serial.Serial(port, BAUD, timeout=0.05)
        self.info = f"直连串口 {port}"

    def _transact(self, cmd: int, data: int) -> tuple[int, bytes] | None:
        """发送 4 字节命令帧，等一帧 7 字节应答；超时返回 None。"""
        self.ser.reset_input_buffer()
        self.ser.write(encode_cmd(cmd, data))
        self.ser.flush()
        buf = bytearray()
        deadline = time.time() + RESPONSE_TIMEOUT
        while time.time() < deadline:
            frame = extract_frame(buf)
            if frame is not None:
                return frame
            chunk = self.ser.read(RESP_LEN)
            if chunk:
                buf.extend(chunk)
        return None

    def handshake(self) -> dict | None:
        """0x00 握手并按判据识别 hub：7 字节定长 + 指令 0x00 + proto=0x01 +
        ch=0x07 + 包尾 0x5A（fw 与通道状态字节不参与判据）。
        通过返回 {"fw", "sw"}，失败返回 None。
        """
        resp = self._transact(CMD_HANDSHAKE, 0x00)
        if resp is None:
            return None
        cmd, data = resp
        if cmd != CMD_HANDSHAKE:
            return None
        fw, proto, ch, sw = data
        if proto != 0x01 or ch != CHANNELS:
            return None
        # bit7 恒 0：防御性屏蔽，与协议 crate 解码层一致
        return {"fw": fw, "sw": sw & 0x7F}

    def read_current(self, ch: int) -> float:
        """0x02 读通道电流，返回 mA。应答指令 = 0xF0 | 通道号，数据 = float32 小端。"""
        resp = self._transact(CMD_CURRENT, ch)
        if resp is None:
            raise HubError(f"{self.port_name} 读 CH{ch} 电流无应答")
        cmd, data = resp
        if cmd != (CURRENT_RESP_BASE | ch):
            raise HubError(f"{self.port_name} 读 CH{ch} 应答指令异常: 0x{cmd:02X}")
        return struct.unpack("<f", data)[0]

    def status(self) -> dict:
        """先握手同步通道状态掩码，再逐通道读电流。
        协议不传电压：恒按 5.000 V 假定，功率 = 5.0 × 电流(A)。
        """
        hs = self.handshake()
        if hs is None:
            raise HubError(f"{self.port_name} 握手失败（不是 power-hub？）")
        sw = hs["sw"]
        channels = []
        for i in range(CHANNELS):
            ch = i + 1
            on = bool(sw & (1 << i))
            # 电压不上传：导通恒按 5.000 V 假定（断开 0V），功率 = 5V × 电流(A)
            voltage = 5.0 if on else 0.0
            current = self.read_current(ch) / 1000.0  # mA → A
            channels.append(
                {
                    "index": ch,
                    "on": on,
                    "voltage": voltage,
                    "current": current,
                    "power": voltage * current,
                    "history": [],
                    "device": None,
                }
            )
        return {
            "connected": True,
            "port": self.port_name,
            "fwVersion": f"0x{hs['fw']:02X}",
            "pollIntervalMs": None,
            "channels": channels,
            "totalPower": sum(c["power"] for c in channels),
            "totalCurrent": sum(c["current"] for c in channels),
            "deviceCount": 0,
        }

    def set_channel(self, ch: int, on: bool) -> None:
        """0x01 通道设置：低 4 位 = 0 基通道索引（0=通道1）+ bit4 开关；应答首字节为切换后 7 位通道掩码。"""
        if not 1 <= ch <= 7:
            raise HubError("通道号范围 1~7")
        resp = self._transact(CMD_CHANNEL, (ch - 1) | (0x10 if on else 0x00))
        if resp is None:
            raise HubError(f"{self.port_name} 通道设置无应答")
        cmd, _sw = resp
        if cmd != CMD_CHANNEL:
            raise HubError(f"{self.port_name} 通道设置应答指令异常: 0x{cmd:02X}")


def is_bluetooth_port(p) -> bool:
    """蓝牙 SPP 口打开会被 Windows 蓝牙栈阻塞数十秒，必须跳过。"""
    desc = (p.description or "").lower()
    return "bluetooth" in desc or "蓝牙" in desc


def select_control_ports(ports):
    """CH340 控制口过滤（scan 与握手扫描共用；语义镜像上位机
    src-tauri/src/serial.rs 的 filter_control_port）。

    已排除蓝牙的候选口按 vid/pid 精确匹配 CH340：命中 → 只留命中口；
    无命中 → 回退全部候选。返回 (候选口, 是否命中)。
    非 USB 口 vid/pid 为 None，天然不命中。
    """
    hits = [p for p in ports if p.vid == CONTROL_VID and p.pid == CONTROL_PID]
    return (hits, True) if hits else (ports, False)


class ProbeSet(NamedTuple):
    """一次串口枚举的探测集：候选口、被跳过的蓝牙口、CH340 是否命中。"""

    ports: list  # 候选口（已排除蓝牙；CH340 命中时仅命中口）
    skipped: list  # 被跳过的蓝牙口
    matched: bool


def close_quietly(transport) -> None:
    """探测结束收尾：关闭串口，忽略已关闭/设备拔出等异常；None（未成功打开）直接跳过。"""
    if transport is None:
        return
    try:
        transport.ser.close()
    except Exception:
        pass


def print_port_filter_note(ports, matched) -> None:
    """过滤结果一行说明，scan 与握手扫描共用。"""
    if matched:
        print(f"  ✓ CH340 控制口命中，只探测：{', '.join(p.device for p in ports)}")
    else:
        print(f"  ○ 无 CH340 命中，回退探测全部 {len(ports)} 个候选口")


def probe_ports() -> ProbeSet:
    """枚举系统串口，排除蓝牙并按 CH340 过滤。依赖 pyserial，未安装时抛 ImportError。"""
    import serial.tools.list_ports

    all_ = list(serial.tools.list_ports.comports())
    physical = [p for p in all_ if not is_bluetooth_port(p)]
    ports, matched = select_control_ports(physical)
    return ProbeSet(ports, [p for p in all_ if is_bluetooth_port(p)], matched)


def find_transport():
    """API 已连接则优先；否则扫描串口直连。"""
    try:
        t = ApiTransport()
        if t.status().get("connected"):
            return t
        api_note = "API 可达但上位机未连接 hub"
    except Exception:
        api_note = "API 不可达（上位机未运行？）"

    try:
        ps = probe_ports()
    except ImportError:
        raise HubError(
            f"{api_note}；直连串口需要 pyserial：uv run --with pyserial scripts/hub.py ..."
        ) from None

    if ps.ports:
        print_port_filter_note(ps.ports, ps.matched)

    errors: list[str] = []
    for p in ps.ports:
        t = None
        try:
            t = SerialTransport(p.device)
            if t.handshake() is not None:
                return t  # 命中：串口保持打开，供后续命令复用
            errors.append(f"{p.device}: 握手判据未通过")
        except Exception as e:
            errors.append(f"{p.device}: {e}")
        # 仅探测失败时关闭（勿放 finally：会连成功路径一起关，返回的连接即失效）
        close_quietly(t)
    detail = "；".join(errors) if errors else "系统无串口"
    raise HubError(f"未找到 hub：{api_note}；串口握手识别失败（{detail}）")


# ---------- 输出 ----------

def fmt_current(a: float) -> str:
    return f"{a * 1000:.1f} mA" if abs(a) < 0.1 else f"{a:.3f} A"


def fmt_power(w: float) -> str:
    return f"{w * 1000:.0f} mW" if abs(w) < 1 else f"{w:.2f} W"


def render_status(st: dict) -> str:
    rows = ["CH  状态  电压      电流       功率      设备"]
    for c in st["channels"]:
        if not c["on"]:
            rows.append(f"{c['index']:<3} OFF  --        --         --        （断电）")
            continue
        dev = c.get("device")
        dev_text = f"{dev['name']} ({dev['vid']:04X}:{dev['pid']:04X})" if dev else "（空）"
        rows.append(
            f"{c['index']:<3} ON   {c['voltage']:.2f} V   {fmt_current(c['current']):>8}   "
            f"{fmt_power(c['power']):>8}  {dev_text}"
        )
    rows.append(
        f"合计：功率 {fmt_power(st['totalPower'])} · 电流 {fmt_current(st['totalCurrent'])} · 设备 {st['deviceCount']}"
    )
    return "\n".join(rows)


# ---------- 子命令 ----------

def cmd_scan(_args: argparse.Namespace) -> None:
    print("探测本地 API …")
    try:
        t = ApiTransport()
        st = t.status()
        if st.get("connected"):
            print(f"  ✓ API 可达，hub 已连接：{st.get('port')}")
        else:
            print("  ○ API 可达，但上位机未连接 hub")
    except Exception:
        print("  ✗ API 不可达（上位机未运行？）")

    print("扫描串口 …")
    try:
        ps = probe_ports()
    except ImportError:
        print("  ! 未安装 pyserial，跳过（uv run --with pyserial …）")
        return
    for p in ps.skipped:
        print(f"  - {p.device}（蓝牙口，跳过）")
    if not ps.ports:
        print("  ✗ 无可探测的实体串口")
        return
    print_port_filter_note(ps.ports, ps.matched)
    for p in ps.ports:
        t = None
        try:
            t = SerialTransport(p.device)
            ok = t.handshake() is not None
            print(f"  {'✓' if ok else '○'} {p.device}{'  ← power-hub' if ok else ''}")
        except Exception as e:
            print(f"  ✗ {p.device}: {e}")
        finally:
            close_quietly(t)  # scan 只报告不保持连接，逐口关闭


def cmd_status(_args: argparse.Namespace) -> None:
    t = find_transport()
    print(f"连接：{t.info}")
    print(render_status(t.status()))


def cmd_set(args: argparse.Namespace, on: bool) -> None:
    t = find_transport()
    t.set_channel(args.ch, on)
    ch = t.status()["channels"][args.ch - 1]
    state = "ON" if ch["on"] else "OFF"
    value = "--" if not ch["on"] else f"{fmt_current(ch['current'])} {fmt_power(ch['power'])}"
    print(f"CH{args.ch} -> {state}  {value}".rstrip())


def cmd_cycle(args: argparse.Namespace) -> None:
    t = find_transport()
    t.set_channel(args.ch, False)
    print(f"CH{args.ch} 已断电，保持 {args.off_secs:g}s …")
    time.sleep(args.off_secs)
    t.set_channel(args.ch, True)
    ch = t.status()["channels"][args.ch - 1]
    state = "ON" if ch["on"] else "OFF"
    print(f"CH{args.ch} 已重新上电：{state}")


def cmd_watch(args: argparse.Namespace) -> None:
    t = find_transport()
    print(f"连接：{t.info}   （Ctrl+C 停止）")
    end = time.time() + args.seconds
    try:
        while time.time() < end:
            st = t.status()
            row = "  ".join(
                f"CH{c['index']}{'●' if c['on'] else '○'}{fmt_current(c['current']) if c['on'] else '  --  ':>9}"
                for c in st["channels"]
            )
            print(f"{time.strftime('%H:%M:%S')}  {row}  |  总 {fmt_power(st['totalPower'])}")
            time.sleep(0.5)
    except KeyboardInterrupt:
        pass


def main() -> None:
    ap = argparse.ArgumentParser(prog="hub.py", description="power-hub 控制（二进制帧协议）")
    sub = ap.add_subparsers(dest="cmd", required=True, metavar="命令")
    sub.add_parser("scan", help="发现 hub：探测本地 API + 扫描串口")
    sub.add_parser("status", help="7 通道遥测、开关态与设备信息")
    p = sub.add_parser("on", help="通道上电")
    p.add_argument("ch", metavar="CH", type=int, choices=range(1, CHANNELS + 1))
    p = sub.add_parser("off", help="通道断电")
    p.add_argument("ch", metavar="CH", type=int, choices=range(1, CHANNELS + 1))
    p = sub.add_parser("cycle", help="电源循环：断电保持 N 秒后重新上电")
    p.add_argument("ch", metavar="CH", type=int, choices=range(1, CHANNELS + 1))
    p.add_argument("--off-secs", type=float, default=2.0)
    p = sub.add_parser("watch", help="连续采样（秒，默认 5）")
    p.add_argument("seconds", nargs="?", type=float, default=5.0)

    args = ap.parse_args()
    handlers = {
        "scan": cmd_scan,
        "status": cmd_status,
        "on": lambda a: cmd_set(a, True),
        "off": lambda a: cmd_set(a, False),
        "cycle": cmd_cycle,
        "watch": cmd_watch,
    }
    try:
        handlers[args.cmd](args)
    except HubError as e:
        print(f"错误：{e}", file=sys.stderr)
        sys.exit(1)


if __name__ == "__main__":
    main()
