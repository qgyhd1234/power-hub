import { useCallback, useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

import ChannelCard from "./components/ChannelCard";
import ConnectionPanel from "./components/ConnectionPanel";
import OverviewBar from "./components/OverviewBar";
import type { HubStatus } from "./types";

export default function App() {
  const [status, setStatus] = useState<HubStatus | null>(null);
  const [dark, setDark] = useState(() => localStorage.getItem("theme") !== "light");
  const [busy, setBusy] = useState(false);
  const [apiError, setApiError] = useState<string | null>(null);

  useEffect(() => {
    const unlisten = listen<HubStatus>("telemetry", (e) => setStatus(e.payload));
    const unlistenApi = listen<string>("api-error", (e) => setApiError(e.payload));
    invoke<HubStatus>("get_status")
      .then(setStatus)
      .catch(() => {});
    return () => {
      unlisten.then((f) => f());
      unlistenApi.then((f) => f());
    };
  }, []);

  useEffect(() => {
    document.documentElement.classList.toggle("dark", dark);
    localStorage.setItem("theme", dark ? "dark" : "light");
  }, [dark]);

  const toggleChannel = useCallback(async (ch: number, on: boolean) => {
    setBusy(true);
    try {
      await invoke("set_channel", { ch, on });
    } catch (e) {
      setApiError(String(e));
    } finally {
      setBusy(false);
    }
  }, []);

  const connected = status?.connected ?? false;

  // 连接后定期重枚举设备树，插拔设备自动反映到通道卡（枚举是注册表级操作，开销极小）
  useEffect(() => {
    if (!connected) return;
    const refresh = () => void invoke("refresh_devices").catch(() => {});
    refresh();
    const timer = setInterval(refresh, 5000);
    return () => clearInterval(timer);
  }, [connected]);

  return (
    <div className="flex min-h-screen flex-col bg-zinc-100 text-zinc-900 dark:bg-zinc-950 dark:text-zinc-100">
      <OverviewBar
        status={status}
        dark={dark}
        onThemeToggle={() => setDark((d) => !d)}
        onIntervalChange={(ms) => void invoke("set_poll_interval", { ms }).catch(() => {})}
        onStatusIntervalChange={(ms) => void invoke("set_status_poll_interval", { ms }).catch(() => {})}
      />

      {apiError && (
        <div className="flex items-center justify-between bg-red-500/10 px-6 py-2 text-sm text-red-600 dark:text-red-400">
          <span>{apiError}</span>
          <button onClick={() => setApiError(null)} aria-label="关闭提示" className="px-1">
            ✕
          </button>
        </div>
      )}

      {connected ? (
        <main className="flex-1 p-6">
          <div className="grid grid-cols-1 gap-4 sm:grid-cols-2 lg:grid-cols-3 xl:grid-cols-4">
            {(status?.channels ?? []).map((ch) => (
              <ChannelCard key={ch.index} ch={ch} connected={connected} busy={busy} onToggle={toggleChannel} />
            ))}
          </div>
        </main>
      ) : (
        <main className="flex flex-1 flex-col">
          <ConnectionPanel
            onConnected={() => {
              invoke<HubStatus>("get_status").then(setStatus).catch(() => {});
            }}
          />
        </main>
      )}

      <footer className="px-6 pb-3 text-center text-[11px] text-zinc-400 dark:text-zinc-600">
        本地 API · 127.0.0.1:8756 · 协议 v1
      </footer>
    </div>
  );
}
