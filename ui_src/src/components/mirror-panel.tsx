import {
  MirrorOutcome,
  groupFailures,
  stoppedMessage,
} from "../upload/mirror-run";
import { FormatBytes } from "../const";
import Button from "./button";

export interface MirrorProgress {
  total: number;
  completed: number;
  failed: number;
  outcome: MirrorOutcome | null;
  finished?: boolean;
}

export interface ServerCoverage {
  url: string;
  hostname: string;
  coveragePercentage: number;
}

export function MirrorErrors({ outcome }: { outcome: MirrorOutcome | null }) {
  if (!outcome || outcome.failed.length === 0) return null;
  const groups = groupFailures(outcome.failed);
  return (
    <div className="bg-red-950 border border-red-900 text-red-200 px-2 py-1 rounded-sm text-xs max-h-48 overflow-y-auto space-y-1">
      {groups.map((g, i) => (
        <details key={i}>
          <summary className="cursor-pointer break-words">
            {g.host}: {g.message} ({g.count}x, {g.files}{" "}
            {g.files === 1 ? "file" : "files"})
          </summary>
          <ul className="pl-3 font-mono break-all">
            {g.failures.map((f, j) => (
              <li key={j}>
                {g.host} {f.op.sha256}: {f.message}
              </li>
            ))}
          </ul>
        </details>
      ))}
      {outcome.stopped.map((server) => (
        <div key={server} className="break-words">
          {stoppedMessage(server)}
        </div>
      ))}
    </div>
  );
}

export interface MirrorPanelViewProps {
  totalFiles: number;
  totalMirrorOperations: number;
  totalSize: number;
  serverCoverage: ServerCoverage[];
  progress: MirrorProgress | null;
  onMirrorAll: () => Promise<void> | void;
  onRetry: () => Promise<void> | void;
}

export function MirrorPanelView({
  totalFiles,
  totalMirrorOperations,
  totalSize,
  serverCoverage,
  progress,
  onMirrorAll,
  onRetry,
}: MirrorPanelViewProps) {
  return (
    <div className="px-3 pb-3 space-y-3 min-w-0">
      <div className="grid grid-cols-3 gap-2 text-center bg-neutral-950 border border-neutral-800 rounded-sm p-2">
        <div className="min-w-0">
          <div className="text-lg font-medium text-white">{totalFiles}</div>
          <div className="text-xs text-neutral-500">Files</div>
        </div>
        <div className="min-w-0">
          <div className="text-lg font-medium text-orange-400">
            {totalMirrorOperations}
          </div>
          <div className="text-xs text-neutral-500">Ops</div>
        </div>
        <div className="min-w-0">
          <div className="text-lg font-medium text-green-400 truncate">
            {FormatBytes(totalSize)}
          </div>
          <div className="text-xs text-neutral-500">Size</div>
        </div>
      </div>

      <div className="grid grid-cols-2 sm:grid-cols-3 lg:grid-cols-4 gap-2">
        {serverCoverage.map((server) => (
          <div
            key={server.url}
            className="bg-neutral-950 border border-neutral-800 rounded-sm p-2 min-w-0"
          >
            <div className="flex items-center justify-between gap-2 mb-1">
              <span className="text-xs text-neutral-300 truncate min-w-0">
                {server.hostname}
              </span>
              <span
                className={`text-xs px-1 rounded-sm shrink-0 ${
                  server.coveragePercentage === 100
                    ? "bg-green-950 text-green-400"
                    : server.coveragePercentage >= 80
                      ? "bg-yellow-950 text-yellow-400"
                      : "bg-red-950 text-red-400"
                }`}
              >
                {server.coveragePercentage}%
              </span>
            </div>
            <div className="w-full bg-neutral-800 rounded-sm h-1">
              <div
                className="bg-white h-1 rounded-sm transition-all"
                style={{ width: `${server.coveragePercentage}%` }}
              />
            </div>
          </div>
        ))}
      </div>

      {!progress ? (
        <Button
          onClick={onMirrorAll}
          disabled={totalMirrorOperations === 0}
          className="w-full"
          size="sm"
        >
          Mirror All
        </Button>
      ) : (
        <div className="space-y-2">
          <div className="w-full bg-neutral-800 rounded-sm h-1">
            <div
              className="bg-white h-1 rounded-sm transition-all"
              style={{
                width: `${((progress.completed + progress.failed) / progress.total) * 100}%`,
              }}
            />
          </div>
          <div className="flex justify-between gap-2 text-xs">
            <span className="text-green-400">{progress.completed} done</span>
            <span className="text-red-400">{progress.failed} failed</span>
            <span className="text-neutral-500">
              {progress.total - progress.completed - progress.failed} left
            </span>
          </div>
          <MirrorErrors outcome={progress.outcome} />
          {progress.finished && progress.failed > 0 && (
            <Button
              onClick={onRetry}
              variant="secondary"
              size="sm"
              className="w-full"
            >
              Retry failed
            </Button>
          )}
        </div>
      )}
    </div>
  );
}
