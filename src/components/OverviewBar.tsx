import { fmtCurrent, fmtPower, type HubStatus } from "../types";

interface OverviewBarProps {
  status: HubStatus | null;
  dark: boolean;
  onThemeToggle: () => void;
  onIntervalChange: (ms: number) => void;
}

const INTERVALS = [250, 500, 1000, 2000];

export default function OverviewBar({ status, dark, onThemeToggle, onIntervalChange }: OverviewBarProps) {
  const connected = status?.connected ?? false;
  return (
    <header className="flex flex-wrap items-center gap-x-6 gap-y-2 border-b border-zinc-200 bg-white px-6 py-3 dark:border-zinc-800 dark:bg-zinc-900/80">
      <div className="flex items-center gap-2">
        <span className="text-lg">⚡</span>
        <span className="text-base font-bold tracking-tight">power-hub</span>
        <span
          className={`ml-2 inline-flex items-center gap-1.5 rounded-full px-2.5 py-0.5 text-xs font-medium ${
            connected
              ? "bg-emerald-100 text-emerald-700 dark:bg-emerald-950 dark:text-emerald-400"
              : "bg-zinc-100 text-zinc-500 dark:bg-zinc-800 dark:text-zinc-400"
          }`}
        >
          <span className={`h-1.5 w-1.5 rounded-full ${connected ? "bg-emerald-500 animate-pulse" : "bg-zinc-400"}`} />
          {connected ? (status?.port ?? "已连接") : "未连接"}
        </span>
        {status?.fwVersion && (
          <span className="hidden font-mono text-[10px] text-zinc-400 dark:text-zinc-600 sm:inline">
            fw {status.fwVersion}
          </span>
        )}
      </div>

      <div className="ml-auto flex items-center gap-x-6 gap-y-2 tabular-nums">
        <Stat label="总功率" value={fmtPower(status?.totalPower ?? 0)} />
        <Stat label="总电流" value={fmtCurrent(status?.totalCurrent ?? 0)} />
        <Stat label="在线设备" value={`${status?.deviceCount ?? 0}`} />

        <label className="flex items-center gap-1.5 text-xs text-zinc-500 dark:text-zinc-400">
          轮询
          <select
            value={status?.pollIntervalMs ?? 500}
            onChange={(e) => onIntervalChange(Number(e.target.value))}
            className="rounded-md border border-zinc-200 bg-zinc-50 px-1.5 py-1 text-xs text-zinc-700 focus:outline-none dark:border-zinc-700 dark:bg-zinc-800 dark:text-zinc-300"
          >
            {INTERVALS.map((ms) => (
              <option key={ms} value={ms}>
                {ms} ms
              </option>
            ))}
          </select>
        </label>

        <button
          onClick={onThemeToggle}
          aria-label="切换主题"
          className="rounded-lg border border-zinc-200 px-2 py-1 text-sm transition-colors hover:bg-zinc-100 dark:border-zinc-700 dark:hover:bg-zinc-800"
        >
          {dark ? "☀" : "☾"}
        </button>
      </div>
    </header>
  );
}

function Stat({ label, value }: { label: string; value: string }) {
  return (
    <div className="text-right">
      <p className="text-[10px] font-medium tracking-wider text-zinc-400 uppercase dark:text-zinc-500">{label}</p>
      <p className="text-sm font-semibold text-zinc-900 dark:text-zinc-100">{value}</p>
    </div>
  );
}
