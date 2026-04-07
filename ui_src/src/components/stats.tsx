import { useState, useEffect } from "react";
import { Route96, DailyStat } from "../upload/admin";
import { FormatBytes } from "../const";

interface StatsProps {
  pub: any;
  url: string;
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

  const maxUploads = Math.max(...stats.map((s) => s.uploads), 1);
  const maxBytes = Math.max(...stats.map((s) => s.bytes), 1);

  return (
    <div className="space-y-4">
      <div className="flex items-center justify-between">
        <h2 className="text-sm font-medium text-white">Upload Stats (Last {days} Days)</h2>
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
            {FormatBytes(stats.reduce((sum, s) => sum + s.bytes, 0), 2)}
          </div>
        </div>
      </div>

      <div className="bg-neutral-900 border border-neutral-800 rounded-sm p-4">
        <h3 className="text-xs text-neutral-500 mb-3">Uploads per Day</h3>
        <div className="flex items-end gap-1 h-40">
          {stats.map((s, i) => {
            const height = (s.uploads / maxUploads) * 100;
            return (
              <div key={i} className="flex-1 flex flex-col items-center group">
                <div className="relative w-full">
                  <div
                    className="bg-blue-600 hover:bg-blue-500 transition-colors rounded-t-sm"
                    style={{ height: `${Math.max(height, 2)}%` }}
                  />
                  <div className="opacity-0 group-hover:opacity-100 absolute bottom-full left-1/2 -translate-x-1/2 mb-1 px-2 py-1 bg-neutral-800 text-xs text-white rounded-sm whitespace-nowrap z-10 pointer-events-none">
                    {s.uploads} uploads
                  </div>
                </div>
              </div>
            );
          })}
        </div>
        <div className="flex justify-between text-xs text-neutral-500 mt-2">
          <span>{stats[0]?.date}</span>
          <span>{stats[stats.length - 1]?.date}</span>
        </div>
      </div>

      <div className="bg-neutral-900 border border-neutral-800 rounded-sm p-4">
        <h3 className="text-xs text-neutral-500 mb-3">Data Uploaded per Day</h3>
        <div className="flex items-end gap-1 h-40">
          {stats.map((s, i) => {
            const height = (s.bytes / maxBytes) * 100;
            return (
              <div key={i} className="flex-1 flex flex-col items-center group">
                <div className="relative w-full">
                  <div
                    className="bg-green-600 hover:bg-green-500 transition-colors rounded-t-sm"
                    style={{ height: `${Math.max(height, 2)}%` }}
                  />
                  <div className="opacity-0 group-hover:opacity-100 absolute bottom-full left-1/2 -translate-x-1/2 mb-1 px-2 py-1 bg-neutral-800 text-xs text-white rounded-sm whitespace-nowrap z-10 pointer-events-none">
                    {FormatBytes(s.bytes, 2)}
                  </div>
                </div>
              </div>
            );
          })}
        </div>
        <div className="flex justify-between text-xs text-neutral-500 mt-2">
          <span>{stats[0]?.date}</span>
          <span>{stats[stats.length - 1]?.date}</span>
        </div>
      </div>
    </div>
  );
}
