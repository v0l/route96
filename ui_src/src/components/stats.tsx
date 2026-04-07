import { useState, useEffect } from "react";
import { Route96, DailyStat } from "../upload/admin";
import { FormatBytes } from "../const";
import {
  BarChart,
  Bar,
  XAxis,
  YAxis,
  CartesianGrid,
  Tooltip,
  ResponsiveContainer,
} from "recharts";

interface StatsProps {
  pub: any;
  url: string;
}

function formatShortDate(date: string) {
  const d = new Date(date + "T00:00:00");
  return `${d.getMonth() + 1}/${d.getDate()}`;
}

function formatShortBytes(value: number) {
  if (value >= 1e9) return `${(value / 1e9).toFixed(1)}G`;
  if (value >= 1e6) return `${(value / 1e6).toFixed(0)}M`;
  if (value >= 1e3) return `${(value / 1e3).toFixed(0)}K`;
  return `${value}`;
}

function UploadsTooltip({ active, payload, label }: any) {
  if (!active || !payload?.length) return null;
  return (
    <div className="bg-neutral-800 border border-neutral-700 px-2 py-1 text-xs text-white rounded-sm">
      <div className="text-neutral-400">{label}</div>
      <div>{payload[0].value.toLocaleString()} uploads</div>
    </div>
  );
}

function BytesTooltip({ active, payload, label }: any) {
  if (!active || !payload?.length) return null;
  return (
    <div className="bg-neutral-800 border border-neutral-700 px-2 py-1 text-xs text-white rounded-sm">
      <div className="text-neutral-400">{label}</div>
      <div>{FormatBytes(payload[0].value, 2)}</div>
    </div>
  );
}

export default function Stats({ pub, url }: StatsProps) {
  const [stats, setStats] = useState<DailyStat[]>([]);
  const [days, setDays] = useState(30);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string>();

  const loadStats = async () => {
    try {
      setLoading(true);
      const route96 = new Route96(url, pub);
      const data = await route96.getStats(days);
      setStats(data.stats);
      setError(undefined);
    } catch (e) {
      setError(e instanceof Error ? e.message : "Failed to load stats");
    } finally {
      setLoading(false);
    }
  };

  useEffect(() => {
    if (pub) {
      loadStats();
    }
  }, [pub, days, url]);

  if (loading) {
    return (
      <div className="flex justify-center items-center h-48">
        <div className="text-sm text-neutral-500">Loading stats...</div>
      </div>
    );
  }

  if (error) {
    return (
      <div className="bg-red-950 border border-red-900 text-red-200 px-3 py-2 rounded-sm text-sm">
        {error}
      </div>
    );
  }

  if (stats.length === 0) {
    return (
      <div className="text-sm text-neutral-500 text-center py-8">
        No stats available for the selected period.
      </div>
    );
  }

  return (
    <div className="space-y-4">
      <div className="flex items-center justify-between">
        <h2 className="text-sm font-medium text-white">
          Upload Stats (Last {days} Days)
        </h2>
        <div className="flex gap-2">
          {[7, 30, 90, 365].map((d) => (
            <button
              key={d}
              onClick={() => setDays(d)}
              className={`px-2 py-1 text-xs rounded-sm ${
                days === d
                  ? "bg-white text-neutral-900"
                  : "bg-neutral-800 text-neutral-300 hover:bg-neutral-700"
              }`}
            >
              {d}d
            </button>
          ))}
        </div>
      </div>

      <div className="grid grid-cols-2 gap-4">
        <div className="bg-neutral-900 border border-neutral-800 rounded-sm p-4">
          <div className="text-xs text-neutral-500 mb-1">Total Uploads</div>
          <div className="text-lg font-medium text-white">
            {stats.reduce((sum, s) => sum + s.uploads, 0).toLocaleString()}
          </div>
        </div>
        <div className="bg-neutral-900 border border-neutral-800 rounded-sm p-4">
          <div className="text-xs text-neutral-500 mb-1">Total Data</div>
          <div className="text-lg font-medium text-white">
            {FormatBytes(
              stats.reduce((sum, s) => sum + s.bytes, 0),
              2,
            )}
          </div>
        </div>
      </div>

      <div className="bg-neutral-900 border border-neutral-800 rounded-sm p-4">
        <h3 className="text-xs text-neutral-500 mb-3">Uploads per Day</h3>
        <div className="h-48">
          <ResponsiveContainer width="100%" height="100%">
            <BarChart data={stats}>
              <CartesianGrid
                strokeDasharray="3 3"
                stroke="#333"
                vertical={false}
              />
              <XAxis
                dataKey="date"
                stroke="#666"
                fontSize={11}
                tickFormatter={formatShortDate}
                interval="preserveStartEnd"
              />
              <YAxis stroke="#666" fontSize={11} width={40} />
              <Tooltip content={<UploadsTooltip />} cursor={{ fill: "#222" }} />
              <Bar dataKey="uploads" fill="#2563eb" radius={[2, 2, 0, 0]} />
            </BarChart>
          </ResponsiveContainer>
        </div>
      </div>

      <div className="bg-neutral-900 border border-neutral-800 rounded-sm p-4">
        <h3 className="text-xs text-neutral-500 mb-3">Data Uploaded per Day</h3>
        <div className="h-48">
          <ResponsiveContainer width="100%" height="100%">
            <BarChart data={stats}>
              <CartesianGrid
                strokeDasharray="3 3"
                stroke="#333"
                vertical={false}
              />
              <XAxis
                dataKey="date"
                stroke="#666"
                fontSize={11}
                tickFormatter={formatShortDate}
                interval="preserveStartEnd"
              />
              <YAxis
                stroke="#666"
                fontSize={11}
                width={45}
                tickFormatter={formatShortBytes}
              />
              <Tooltip content={<BytesTooltip />} cursor={{ fill: "#222" }} />
              <Bar dataKey="bytes" fill="#16a34a" radius={[2, 2, 0, 0]} />
            </BarChart>
          </ResponsiveContainer>
        </div>
      </div>
    </div>
  );
}
