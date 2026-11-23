import { useState, useEffect, useCallback, useMemo, useRef } from "react";
import { Blossom } from "../upload/blossom";
import { MirrorOp, runMirror } from "../upload/mirror-run";
import Button from "./button";
import { MirrorPanelView, MirrorProgress } from "./mirror-panel";
import usePublisher from "../hooks/publisher";
import useLogin from "../hooks/login";

interface FileMirrorSuggestion {
  sha256: string;
  url: string;
  size: number;
  mime_type?: string;
  available_on: string[];
  missing_from: string[];
}

interface MirrorSuggestionsProps {
  servers: string[];
}

export default function MirrorSuggestions({ servers }: MirrorSuggestionsProps) {
  const [suggestions, setSuggestions] = useState<FileMirrorSuggestion[]>([]);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string>();
  const [mirrorAllProgress, setMirrorAllProgress] =
    useState<MirrorProgress | null>(null);
  const [isExpanded, setIsExpanded] = useState(false);
  const [hasFetched, setHasFetched] = useState(false);

  const pub = usePublisher();
  const login = useLogin();

  const prevServersRef = useRef<string[]>([]);

  const memoizedServers = useMemo(() => {
    const sortedServers = [...servers].sort();
    const prevSorted = [...prevServersRef.current].sort();

    if (
      sortedServers.length === prevSorted.length &&
      sortedServers.every((s, i) => s === prevSorted[i])
    ) {
      return prevServersRef.current;
    }

    prevServersRef.current = servers;
    return servers;
  }, [servers]);

  const fetchSuggestions = useCallback(async () => {
    if (!pub || !login?.publicKey) return;

    try {
      setLoading(true);
      setError(undefined);

      const serverList = [...memoizedServers];

      if (serverList.length <= 1) {
        setLoading(false);
        return;
      }

      const fileMap: Map<string, FileMirrorSuggestion> = new Map();

      for (const serverUrl of serverList) {
        try {
          const blossom = new Blossom(serverUrl, pub);
          const files = await blossom.list(login.publicKey);

          for (const file of files) {
            const suggestion = fileMap.get(file.sha256);
            if (suggestion) {
              suggestion.available_on.push(serverUrl);
            } else {
              fileMap.set(file.sha256, {
                sha256: file.sha256,
                url: file.url || "",
                size: file.size,
                mime_type: file.type,
                available_on: [serverUrl],
                missing_from: [],
              });
            }
          }
        } catch (e) {
          console.error(`Failed to fetch files from ${serverUrl}:`, e);
        }
      }

      for (const suggestion of fileMap.values()) {
        for (const serverUrl of serverList) {
          if (!suggestion.available_on.includes(serverUrl)) {
            suggestion.missing_from.push(serverUrl);
          }
        }
      }

      const filteredSuggestions = Array.from(fileMap.values()).filter(
        (s) => s.missing_from.length > 0 && s.available_on.length > 0,
      );

      setSuggestions(filteredSuggestions);
    } catch (e) {
      if (e instanceof Error) {
        setError(e.message);
      } else {
        setError("Failed to fetch mirror suggestions");
      }
    } finally {
      setLoading(false);
    }
  }, [memoizedServers, pub, login?.publicKey]);

  useEffect(() => {
    if (isExpanded && memoizedServers.length > 1 && pub && login?.publicKey) {
      fetchSuggestions();
      setHasFetched(true);
    }
  }, [isExpanded, memoizedServers, pub, login?.publicKey, fetchSuggestions]);

  async function runOps(ops: MirrorOp[]) {
    if (!pub || ops.length === 0) return;
    setMirrorAllProgress({
      total: ops.length,
      completed: 0,
      failed: 0,
      outcome: null,
    });
    const outcome = await runMirror(
      ops,
      async (op) => {
        await new Blossom(op.server, pub).mirror(op.url, op.sha256);
        setSuggestions((prev) =>
          prev
            .map((s) =>
              s.sha256 === op.sha256
                ? {
                    ...s,
                    available_on: [...s.available_on, op.server],
                    missing_from: s.missing_from.filter(
                      (server) => server !== op.server,
                    ),
                  }
                : s,
            )
            .filter((s) => s.missing_from.length > 0),
        );
      },
      (o) =>
        setMirrorAllProgress({
          total: ops.length,
          completed: o.completed.length,
          failed: o.failed.length,
          outcome: { ...o, failed: [...o.failed] },
        }),
    );
    setMirrorAllProgress({
      total: ops.length,
      completed: outcome.completed.length,
      failed: outcome.failed.length,
      outcome,
      finished: true,
    });
    if (outcome.failed.length === 0) {
      setTimeout(() => setMirrorAllProgress(null), 3000);
    }
  }

  async function mirrorAll() {
    const ops: MirrorOp[] = suggestions.flatMap((s) =>
      s.missing_from.map((server) => ({
        sha256: s.sha256,
        url: s.url,
        server,
      })),
    );
    await runOps(ops);
  }

  async function retryFailed() {
    const failed = mirrorAllProgress?.outcome?.failed ?? [];
    await runOps(failed.map((f) => f.op));
  }

  const totalFiles = suggestions.length;
  const totalMirrorOperations = suggestions.reduce(
    (total, suggestion) => total + suggestion.missing_from.length,
    0,
  );
  const totalSize = suggestions.reduce(
    (total, suggestion) => total + suggestion.size,
    0,
  );

  const serverCoverage = memoizedServers.map((serverUrl) => {
    const filesOnServer = suggestions.filter((s) =>
      s.available_on.includes(serverUrl),
    ).length;
    const totalFilesAcrossAllServers = new Set(suggestions.map((s) => s.sha256))
      .size;
    const coveragePercentage =
      totalFilesAcrossAllServers > 0
        ? Math.round((filesOnServer / totalFilesAcrossAllServers) * 100)
        : 100;

    return {
      url: serverUrl,
      hostname: new URL(serverUrl).hostname,
      filesCount: filesOnServer,
      totalFiles: totalFilesAcrossAllServers,
      coveragePercentage,
    };
  });

  if (memoizedServers.length <= 1) {
    return null;
  }

  const CollapsibleHeader = ({
    title,
    badge,
  }: {
    title: string;
    badge?: React.ReactNode;
  }) => (
    <button
      onClick={() => setIsExpanded(!isExpanded)}
      className="w-full flex items-center justify-between p-2 text-left hover:bg-neutral-800/50 transition-colors"
    >
      <div className="flex items-center gap-2">
        <svg
          className={`w-3 h-3 text-neutral-500 transition-transform ${isExpanded ? "rotate-90" : ""}`}
          fill="none"
          stroke="currentColor"
          viewBox="0 0 24 24"
        >
          <path
            strokeLinecap="round"
            strokeLinejoin="round"
            strokeWidth={2}
            d="M9 5l7 7-7 7"
          />
        </svg>
        <span className="text-sm font-medium text-white">{title}</span>
        {badge}
      </div>
    </button>
  );

  if (!hasFetched && !loading) {
    return (
      <div className="bg-neutral-900 border border-neutral-800 rounded-sm">
        <CollapsibleHeader title="Mirror" />
      </div>
    );
  }

  if (loading) {
    return (
      <div className="bg-neutral-900 border border-neutral-800 rounded-sm">
        <CollapsibleHeader title="Mirror" />
        {isExpanded && (
          <div className="px-3 pb-2">
            <span className="text-xs text-neutral-500">Loading...</span>
          </div>
        )}
      </div>
    );
  }

  if (error) {
    return (
      <div className="bg-neutral-900 border border-neutral-800 rounded-sm">
        <CollapsibleHeader
          title="Mirror"
          badge={
            <span className="px-1.5 py-0.5 text-xs bg-red-950 text-red-400 rounded-sm">
              Error
            </span>
          }
        />
        {isExpanded && (
          <div className="px-3 pb-2 space-y-2">
            <div className="bg-red-950 border border-red-900 text-red-200 px-2 py-1 rounded-sm text-xs">
              {error}
            </div>
            <Button
              onClick={() => {
                setHasFetched(false);
                fetchSuggestions();
              }}
              variant="secondary"
              size="sm"
            >
              Retry
            </Button>
          </div>
        )}
      </div>
    );
  }

  if (suggestions.length === 0) {
    return (
      <div className="bg-neutral-900 border border-neutral-800 rounded-sm">
        <CollapsibleHeader
          title="Mirror"
          badge={
            <span className="px-1.5 py-0.5 text-xs bg-green-950 text-green-400 rounded-sm">
              Synced
            </span>
          }
        />
        {isExpanded && (
          <div className="px-3 pb-2">
            <span className="text-xs text-neutral-500">
              All files synchronized.
            </span>
          </div>
        )}
      </div>
    );
  }

  return (
    <div className="bg-neutral-900 border border-neutral-800 rounded-sm">
      <CollapsibleHeader
        title="Mirror"
        badge={
          <span className="px-1.5 py-0.5 text-xs bg-orange-950 text-orange-400 rounded-sm">
            {totalMirrorOperations} ops
          </span>
        }
      />
      {isExpanded && (
        <MirrorPanelView
          totalFiles={totalFiles}
          totalMirrorOperations={totalMirrorOperations}
          totalSize={totalSize}
          serverCoverage={serverCoverage}
          progress={mirrorAllProgress}
          onMirrorAll={mirrorAll}
          onRetry={retryFailed}
        />
      )}
    </div>
  );
}
