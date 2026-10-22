import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import type { PortInfo, PortList } from "../types";

interface ConnectionPanelProps {
  onConnected: () => void;
}

function portLabel(p: PortInfo): string {
  if (p.vid == null || p.pid == null) return p.name;
  const hex = (n: number) => n.toString(16).toUpperCase().padStart(4, "0");
  return `${p.name}（${hex(p.vid)}:${hex(p.pid)}）`;
}

export default function ConnectionPanel({ onConnected }: ConnectionPanelProps) {
  const [ports, setPorts] = useState<PortInfo[]>([]);
  const [filtered, setFiltered] = useState(false);
  const [selected, setSelected] = useState<string>("");
  const [scanning, setScanning] = useState(false);
  const [connecting, setConnecting] = useState(false);
  const [error, setError] = useState<string | null>(null);

  async function scan(selectFirst: boolean) {
    setScanning(true);
    setError(null);
    try {
      const list = await invoke<PortList>("list_ports");
      setPorts(list.ports);
      setFiltered(list.filtered);
      if (selectFirst && list.ports.length > 0) {
        setSelected((cur) => cur || list.ports[0].name);
      }
      if (list.ports.length === 0) {
        setError("未发现串口设备，请检查 USB 连接");
      }
    } catch (e) {
      setError(String(e));
    } finally {
      setScanning(false);
    }
  }

  useEffect(() => {
    void scan(true);
  }, []);

  async function connect() {
    if (!selected) return;
    setConnecting(true);
    setError(null);
    try {
      await invoke("connect", { port: selected });
      onConnected();
    } catch (e) {
      setError(String(e));
    } finally {
      setConnecting(false);
    }
  }

  async function startDemo() {
    setError(null);
    try {
      await invoke("start_demo");
      onConnected();
    } catch (e) {
      setError(String(e));
    }
  }

  return (
    <div className="flex flex-1 flex-col items-center justify-center gap-5">
      <div className="text-center">
        <p className="text-2xl font-bold tracking-tight">连接 power-hub</p>
        <p className="mt-1 text-sm text-zinc-500 dark:text-zinc-400">
          上位机将扫描串口并自动探活（115200 · 协议 v1）
        </p>
      </div>

      <div className="flex items-center gap-2">
        <select
          value={selected}
          onChange={(e) => setSelected(e.target.value)}
          className="w-44 rounded-lg border border-zinc-300 bg-white px-3 py-2 text-sm focus:outline-none focus-visible:ring-2 focus-visible:ring-amber-500/60 dark:border-zinc-700 dark:bg-zinc-900"
        >
          {ports.length === 0 && <option value="">（无可用串口）</option>}
          {ports.map((p) => (
            <option key={p.name} value={p.name}>
              {portLabel(p)}
            </option>
          ))}
        </select>
        <button
          onClick={() => scan(false)}
          disabled={scanning}
          className="rounded-lg border border-zinc-300 px-3 py-2 text-sm transition-colors hover:bg-zinc-100 disabled:opacity-50 dark:border-zinc-700 dark:hover:bg-zinc-800"
        >
          {scanning ? "扫描中…" : "扫描"}
        </button>
        <button
          onClick={connect}
          disabled={!selected || connecting}
          className="rounded-lg bg-amber-500 px-4 py-2 text-sm font-semibold text-zinc-950 transition-colors hover:bg-amber-400 disabled:cursor-not-allowed disabled:opacity-50"
        >
          {connecting ? "连接中…" : "连接"}
        </button>
      </div>

      <button
        onClick={startDemo}
        disabled={connecting}
        className="rounded-lg border border-dashed border-zinc-300 px-4 py-1.5 text-xs text-zinc-500 transition-colors hover:border-amber-400 hover:text-amber-600 disabled:opacity-50 dark:border-zinc-700 dark:text-zinc-400 dark:hover:border-amber-500 dark:hover:text-amber-400"
      >
        没有硬件？进入演示模式 →
      </button>

      {!filtered && ports.length > 0 && (
        <p className="text-xs text-zinc-400 dark:text-zinc-500">
          未发现 CH340 串口，已展示全部串口
        </p>
      )}

      {error && <p className="text-sm text-red-500">{error}</p>}
    </div>
  );
}
