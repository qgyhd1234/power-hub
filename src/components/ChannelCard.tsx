import Sparkline from "./Sparkline";
import Switch from "./Switch";
import { fmtCurrent, fmtPower, fmtVidPid, fmtVoltage, type ChannelStatus } from "../types";

interface ChannelCardProps {
  ch: ChannelStatus;
  connected: boolean;
  busy: boolean;
  onToggle: (ch: number, on: boolean) => void;
}

export default function ChannelCard({ ch, connected, busy, onToggle }: ChannelCardProps) {
  const dimmed = connected && !ch.on;
  return (
    <div
      className={`flex flex-col gap-3 rounded-2xl border p-4 transition-all duration-200 ${
        dimmed
          ? "border-zinc-200 bg-zinc-50 dark:border-zinc-800/60 dark:bg-zinc-900/40"
          : "border-zinc-200 bg-white shadow-sm dark:border-zinc-800 dark:bg-zinc-900"
      } ${!connected ? "opacity-50" : ""}`}
    >
      <div className="flex items-center justify-between">
        <span className="text-sm font-semibold tracking-wider text-zinc-500 dark:text-zinc-400">
          CH {ch.index}
        </span>
        <Switch
          checked={ch.on}
          disabled={!connected || busy}
          onChange={(on) => onToggle(ch.index, on)}
          aria-label={`通道 ${ch.index} 电源`}
        />
      </div>

      <div className="grid grid-cols-3 gap-1 tabular-nums">
        <Metric label="电压" value={dimmed ? "--" : fmtVoltage(ch.voltage)} />
        <Metric label="电流" value={dimmed ? "--" : fmtCurrent(ch.current)} />
        <Metric label="功率" value={dimmed ? "--" : fmtPower(ch.power)} />
      </div>

      <Sparkline data={dimmed ? [] : ch.history} />

      <div className="min-h-8 border-t border-zinc-100 pt-2 dark:border-zinc-800/60">
        {dimmed ? (
          <p className="text-xs text-zinc-400 dark:text-zinc-600">已断电</p>
        ) : ch.device ? (
          <div className="flex items-baseline justify-between gap-2">
            <span className="truncate text-xs font-medium text-zinc-700 dark:text-zinc-300">
              {ch.device.name}
            </span>
            <span className="shrink-0 font-mono text-[10px] text-zinc-400 dark:text-zinc-500">
              {fmtVidPid(ch.device)}
            </span>
          </div>
        ) : (
          <p className="text-xs text-zinc-400 dark:text-zinc-600">（空）</p>
        )}
      </div>
    </div>
  );
}

function Metric({ label, value }: { label: string; value: string }) {
  return (
    <div>
      <p className="text-[10px] font-medium tracking-wider text-zinc-400 uppercase dark:text-zinc-500">
        {label}
      </p>
      <p className="truncate text-base font-semibold text-zinc-900 dark:text-zinc-100">{value}</p>
    </div>
  );
}
