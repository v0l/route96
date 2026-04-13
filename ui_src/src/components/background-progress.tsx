import { useState, useEffect } from "react";
import { Route96, BackgroundProgressResponse } from "../upload/admin";

interface BackgroundProgressProps {
  pub: any;
  url: string;
}

function TaskRow({ task }: { task: BackgroundProgressResponse["tasks"][0] }) {
  const label = task.task.replace(/^labels:/, "");
  const isLabels = task.task.startsWith("labels:");
  const percent = Math.min(100, task.percent);

  let barColor = "bg-white";
  if (percent < 50) barColor = "bg-red-400";
  else if (percent < 90) barColor = "bg-yellow-400";

  return (
    <div className="bg-neutral-900 border border-neutral-800 rounded-sm p-3">
      <div className="flex items-center justify-between mb-2">
        <span className="text-sm text-neutral-300 font-medium">
          {isLabels ? `Labeling: ${label}` : label.replace(/_/g, " ").replace(/\b\w/g, (c) => c.toUpperCase())}
        </span>
        <span className="text-xs text-neutral-500">
          {task.total - task.pending} / {task.total} &middot; {percent.toFixed(1)}%
        </span>
      </div>
      <div className="w-full bg-neutral-800 rounded-sm h-1.5">
        <div
          className={`${barColor} h-1.5 rounded-sm transition-all`}
          style={{ width: `${percent}%` }}
        />
      </div>
      <div className="mt-1 text-xs text-neutral-600">
        {task.pending.toLocaleString()} pending
      </div>
    </div>
  );
}

export default function BackgroundProgress({ pub, url }: BackgroundProgressProps) {
  const [data, setData] = useState<BackgroundProgressResponse>();
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string>();

  const loadProgress = async () => {
    try {
      setLoading(true);
      const route96 = new Route96(url, pub);
      const result = await route96.getBackgroundProgress();
      setData(result);
      setError(undefined);
    } catch (e) {
      setError(e instanceof Error ? e.message : "Failed to load background progress");
    } finally {
      setLoading(false);
    }
  };

  useEffect(() => {
    if (pub) {
      loadProgress();
    }
  }, [pub, url]);

  if (loading) {
    return (
      <div className="flex justify-center items-center h-48">
        <div className="text-sm text-neutral-500">Loading progress...</div>
      </div>
    );
  }

  if (error) {
    return (
      <div className="bg-red-950 border border-red-900 text-red-200 px-3 py-2 rounded-sm text-sm">
        {error}
        <button
          onClick={loadProgress}
          className="ml-2 underline hover:text-red-100"
        >
          Retry
        </button>
      </div>
    );
  }

  if (!data) return null;

  const totalPercent = Math.min(100, data.total_percent);
  let totalBarColor = "bg-white";
  if (totalPercent < 50) totalBarColor = "bg-red-400";
  else if (totalPercent < 90) totalBarColor = "bg-yellow-400";

  return (
    <div className="space-y-4">
      {/* Overall summary */}
      <div className="bg-neutral-900 border border-neutral-800 rounded-sm p-4">
        <div className="flex items-center justify-between mb-2">
          <span className="text-sm text-white font-medium">Overall Progress</span>
          <span className="text-xs text-neutral-500">
            {totalPercent.toFixed(1)}%
          </span>
        </div>
        <div className="w-full bg-neutral-800 rounded-sm h-2">
          <div
            className={`${totalBarColor} h-2 rounded-sm transition-all`}
            style={{ width: `${totalPercent}%` }}
          />
        </div>
        <div className="mt-1 text-xs text-neutral-500">
          {data.total_pending.toLocaleString()} files pending across {data.tasks.length} task{data.tasks.length === 1 ? "" : "s"}
        </div>
      </div>

      {/* Per-task breakdown */}
      {data.tasks.length > 0 ? (
        <div className="space-y-2">
          {data.tasks.map((t) => (
            <TaskRow key={t.task} task={t} />
          ))}
        </div>
      ) : (
        <div className="text-sm text-neutral-500 text-center py-8">
          No background tasks running.
        </div>
      )}

      <div className="flex justify-center">
        <button
          onClick={loadProgress}
          className="bg-neutral-800 hover:bg-neutral-700 text-white px-3 py-1.5 rounded-sm text-xs"
        >
          Refresh
        </button>
      </div>
    </div>
  );
}
