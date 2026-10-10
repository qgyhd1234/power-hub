import { AreaChart, Area, ResponsiveContainer, YAxis } from "recharts";

interface SparklineProps {
  data: number[];
}

/** 通道电流迷你曲线（最近 ~60s）。 */
export default function Sparkline({ data }: SparklineProps) {
  if (data.length < 2) {
    return (
      <div className="flex h-14 items-center justify-center rounded-lg bg-zinc-50 text-xs text-zinc-400 dark:bg-zinc-950/60 dark:text-zinc-600">
        暂无采样
      </div>
    );
  }
  const points = data.map((v, i) => ({ i, v }));
  return (
    <div className="h-14 rounded-lg bg-zinc-50 dark:bg-zinc-950/60">
      <ResponsiveContainer width="100%" height="100%">
        <AreaChart data={points} margin={{ top: 4, right: 4, bottom: 4, left: 4 }}>
          <defs>
            <linearGradient id="sparkFill" x1="0" y1="0" x2="0" y2="1">
              <stop offset="0%" stopColor="#f59e0b" stopOpacity={0.35} />
              <stop offset="100%" stopColor="#f59e0b" stopOpacity={0} />
            </linearGradient>
          </defs>
          <YAxis hide domain={[0, "auto"]} />
          <Area
            type="monotone"
            dataKey="v"
            stroke="#f59e0b"
            strokeWidth={1.5}
            fill="url(#sparkFill)"
            dot={false}
            isAnimationActive={false}
          />
        </AreaChart>
      </ResponsiveContainer>
    </div>
  );
}
