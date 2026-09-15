import {
  Area,
  AreaChart,
  Bar,
  BarChart,
  CartesianGrid,
  Cell,
  Pie,
  PieChart,
  ResponsiveContainer,
  Tooltip,
  XAxis,
  YAxis,
} from 'recharts';

const axisProps = {
  stroke: 'var(--color-muted-foreground)',
  fontSize: 11,
  tickLine: false,
  axisLine: false,
} as const;

const tooltipStyle = {
  contentStyle: {
    background: 'var(--color-popover)',
    border: '1px solid var(--color-border)',
    borderRadius: '8px',
    fontSize: '12px',
    color: 'var(--color-popover-foreground)',
  },
  labelStyle: { color: 'var(--color-muted-foreground)' },
  itemStyle: { color: 'var(--color-popover-foreground)' },
} as const;

export type RequestPoint = { t: string; http2: number; http3: number };
export type BandwidthPoint = { t: string; ingress: number; egress: number };
export type StatusMixItem = { name: string; value: number; color: string };

export function RequestsChart({ data }: { data: RequestPoint[] }) {
  return (
    <ResponsiveContainer width="100%" height={240}>
      <AreaChart data={data} margin={{ left: -18, right: 4, top: 8 }}>
        <defs>
          <linearGradient id="fillH3" x1="0" y1="0" x2="0" y2="1">
            <stop offset="0%" stopColor="var(--color-chart-1)" stopOpacity={0.4} />
            <stop offset="100%" stopColor="var(--color-chart-1)" stopOpacity={0} />
          </linearGradient>
          <linearGradient id="fillH2" x1="0" y1="0" x2="0" y2="1">
            <stop offset="0%" stopColor="var(--color-chart-2)" stopOpacity={0.35} />
            <stop offset="100%" stopColor="var(--color-chart-2)" stopOpacity={0} />
          </linearGradient>
        </defs>
        <CartesianGrid strokeDasharray="3 3" stroke="var(--color-border)" vertical={false} />
        <XAxis dataKey="t" {...axisProps} />
        <YAxis {...axisProps} />
        <Tooltip {...tooltipStyle} />
        <Area
          type="monotone"
          dataKey="http3"
          name="HTTP/3"
          stroke="var(--color-chart-1)"
          strokeWidth={2}
          fill="url(#fillH3)"
        />
        <Area
          type="monotone"
          dataKey="http2"
          name="HTTP/2"
          stroke="var(--color-chart-2)"
          strokeWidth={2}
          fill="url(#fillH2)"
        />
      </AreaChart>
    </ResponsiveContainer>
  );
}

export function BandwidthChart({ data, unit = 'MB/s' }: { data: BandwidthPoint[]; unit?: string }) {
  return (
    <ResponsiveContainer width="100%" height={240}>
      <BarChart data={data} margin={{ left: -18, right: 4, top: 8 }}>
        <CartesianGrid strokeDasharray="3 3" stroke="var(--color-border)" vertical={false} />
        <XAxis dataKey="t" {...axisProps} />
        <YAxis {...axisProps} unit={unit ? ` ${unit}` : undefined} width={56} />
        <Tooltip {...tooltipStyle} cursor={{ fill: 'var(--color-muted)', opacity: 0.4 }} />
        <Bar dataKey="egress" name="Egress" fill="var(--color-chart-1)" radius={[3, 3, 0, 0]} />
        <Bar dataKey="ingress" name="Ingress" fill="var(--color-chart-2)" radius={[3, 3, 0, 0]} />
      </BarChart>
    </ResponsiveContainer>
  );
}

export function StatusMixChart({ data }: { data: StatusMixItem[] }) {
  return (
    <ResponsiveContainer width="100%" height={220}>
      <PieChart>
        <Pie
          data={data}
          dataKey="value"
          nameKey="name"
          innerRadius={58}
          outerRadius={88}
          paddingAngle={2}
          strokeWidth={0}
        >
          {data.map((entry) => (
            <Cell key={entry.name} fill={entry.color} />
          ))}
        </Pie>
        <Tooltip {...tooltipStyle} formatter={(v: number) => `${v}%`} />
      </PieChart>
    </ResponsiveContainer>
  );
}

export function ChartLegend({ items }: { items: { label: string; color: string }[] }) {
  return (
    <div className="flex flex-wrap items-center gap-x-4 gap-y-1.5">
      {items.map((item) => (
        <div key={item.label} className="flex items-center gap-1.5">
          <span className="size-2.5 rounded-full" style={{ background: item.color }} />
          <span className="text-xs text-muted-foreground">{item.label}</span>
        </div>
      ))}
    </div>
  );
}
