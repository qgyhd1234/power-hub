---
name: power-hub
description: 通过串口/本地API控制 power-hub USB Hub（1转7 USB3.0，7通道独立电源）：查询每通道电压/电流/功率遥测、开关通道电源、电源循环重启 USB 设备、监控端口接入设备。当用户提到 power-hub、USB Hub 电源、通道通断、给某个USB口断电/上电、重启USB设备、USB功率监控时使用。
---

# power-hub 控制

## 概览

- 硬件：1 转 7 USB3.0 hub，7 个通道的 VBUS 独立可控，串口 115200 8N1 文本行协议 v1。
- **所有操作通过 `scripts/hub.py` 完成**，不要手写临时脚本。
- 连接自动降级：优先上位机本地 API（`127.0.0.1:8756`，上位机运行中时可用，含设备信息），否则直连串口（仅遥测与开关）。
- 统一运行前缀：`uv run --with pyserial scripts/hub.py …`

## 会话开始：先探活

```bash
uv run --with pyserial scripts/hub.py scan     # 第一步：发现 hub
uv run --with pyserial scripts/hub.py status   # 第二步：查看现状，避免覆盖用户配置的通道状态
```

## 子命令速查

| 命令                            | 作用                                     |
| ------------------------------- | ---------------------------------------- |
| `scan`                          | 探测本地 API + 逐个串口 PING 探活        |
| `status`                        | 7 通道遥测、开关态、设备信息             |
| `on CH` / `off CH`              | 通道电源开关（CH = 1~7）                 |
| `cycle CH [--off-secs N]`       | 电源循环重启（默认断电保持 2s）          |
| `watch [SECONDS]`               | 连续采样电流/功率（默认 5s，0.5s 一行）  |

## 高频工作流

1. **远程重启死机的 USB 设备**：`status` 找到通道 → `cycle 3` → `status` 确认恢复。
2. **监控某设备功耗**：`watch 10`，观察电流/功率走势。
3. **给某口断电保护**：`off 5` → `status` 确认该行显示 OFF。
4. **批量下电**：逐个 `off 1` … `off 7`（协议 v1 无批量命令）。

## 稳健性规则

- ⚠ 本技能全部命令**待实机联调**（硬件尚未到手），首次上硬件时逐条验证。
- 上位机运行且已连接时**必须走 API**（串口被上位机独占）；`scan` 会自动识别。
- 通道 `off` 后设备掉线，`status` 中该通道设备列显示"（断电）"是正常现象；重新 `on` 后设备信息需上位机重新枚举，可能延迟数秒。
- 命令失败先跑 `scan` 确认连接方式，再查上位机是否占用串口。
- 协议细节读仓库 `docs/protocol.md`；摘要见 [references/protocol-summary.md](./references/protocol-summary.md)。
