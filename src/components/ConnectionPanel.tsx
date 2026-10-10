import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";

interface ConnectionPanelProps {
  onConnected: () => void;
}

export default function ConnectionPanel({ onConnected }: ConnectionPanelProps) {
  const [ports, setPorts] = useState<string[]>([]);
  const [selected, setSelected] = useState<string>("");
  const [scanning, setScanning] = useState(false);
  const [connecting, setConnecting] = useState(false);
  const [error, setError] = useState<string | null>(null);

  async function scan(selectFirst: boolean) {
    setScanning(true);
    setError(null);
    try {
      const list = await invoke<string[]>("list_ports");
      setPorts(list);
      if (selectFirst && list.length > 0) {
        setSelected((cur) => (cur || list[0]));
      }
      if (list.length === 0) {
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
            <option key={p} value={p}>
              {p}
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

      {error && <p className="text-sm text-red-500">{error}</p>}
    </div>
  );
}
