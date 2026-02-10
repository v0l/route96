import { useState, useEffect, useCallback, useMemo, useRef } from "react";
import { Blossom } from "../upload/blossom";
import { FormatBytes } from "../const";
import Button from "./button";
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

interface MirrorProgress {
  total: number;
  completed: number;
  failed: number;
  errors: string[];
}

export default function MirrorSuggestions({ servers }: MirrorSuggestionsProps) {
  const [suggestions, setSuggestions] = useState<FileMirrorSuggestion[]>([]);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string>();
  const [mirrorAllProgress, setMirrorAllProgress] = useState<MirrorProgress | null>(null);
  const [isExpanded, setIsExpanded] = useState(false);
  const [hasFetched, setHasFetched] = useState(false);

  const pub = usePublisher();
  const login = useLogin();

  const prevServersRef = useRef<string[]>([]);
  
  const memoizedServers = useMemo(() => {
    const sortedServers = [...servers].sort();
    const prevSorted = [...prevServersRef.current].sort();
    
    if (sortedServers.length === prevSorted.length && 
        sortedServers.every((s, i) => s === prevSorted[i])) {
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
        s => s.missing_from.length > 0 && s.available_on.length > 0
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
    if (isExpanded && memoizedServers.length > 1 && pub && login?.publicKey && !hasFetched) {
      fetchSuggestions();
      setHasFetched(true);
    }
  }, [isExpanded, memoizedServers, pub, login?.publicKey, fetchSuggestions, hasFetched]);

  async function mirrorAll() {
    if (!pub || suggestions.length === 0) return;

    const totalOperations = suggestions.reduce((total, suggestion) =>
      total + suggestion.missing_from.length, 0
    );

    setMirrorAllProgress({
      total: totalOperations,
      completed: 0,
      failed: 0,
      errors: []
    });

    let completed = 0;
    let failed = 0;
    const errors: string[] = [];

    for (const suggestion of suggestions) {
      for (const targetServer of suggestion.missing_from) {
        try {
          const blossom = new Blossom(targetServer, pub);
          await blossom.mirror(suggestion.url);
          completed++;

          setMirrorAllProgress(prev => prev ? {
            ...prev,
            completed: completed,
            failed: failed
          } : null);

          setSuggestions(prev =>
            prev.map(s =>
              s.sha256 === suggestion.sha256
                ? {
                  ...s,
                  available_on: [...s.available_on, targetServer],
                  missing_from: s.missing_from.filter(server => server !== targetServer)
                }
                : s
            ).filter(s => s.missing_from.length > 0)
          );
        } catch (e) {
          failed++;
          const errorMessage = e instanceof Error ? e.message : "Unknown error";
          const serverHost = new URL(targetServer).hostname;
          errors.push(`${serverHost}: ${errorMessage}`);

          setMirrorAllProgress(prev => prev ? {
            ...prev,
            completed: completed,
            failed: failed,
            errors: [...errors]
          } : null);
        }
      }
    }

    setTimeout(() => {
      setMirrorAllProgress(null);
    }, 3000);
  }

  const totalFiles = suggestions.length;
  const totalMirrorOperations = suggestions.reduce((total, suggestion) =>
    total + suggestion.missing_from.length, 0
  );
  const totalSize = suggestions.reduce((total, suggestion) => total + suggestion.size, 0);

  const serverCoverage = memoizedServers.map(serverUrl => {
    const filesOnServer = suggestions.filter(s => s.available_on.includes(serverUrl)).length;
    const totalFilesAcrossAllServers = new Set(suggestions.map(s => s.sha256)).size;
    const coveragePercentage = totalFilesAcrossAllServers > 0 ?
      Math.round((filesOnServer / totalFilesAcrossAllServers) * 100) : 100;

    return {
      url: serverUrl,
      hostname: new URL(serverUrl).hostname,
      filesCount: filesOnServer,
      totalFiles: totalFilesAcrossAllServers,
      coveragePercentage
    };
  });

  if (memoizedServers.length <= 1) {
    return null;
  }

  const CollapsibleHeader = ({ title, badge }: { title: string; badge?: React.ReactNode }) => (
    <button
      onClick={() => setIsExpanded(!isExpanded)}
      className="w-full flex items-center justify-between p-2 text-left hover:bg-neutral-800/50 transition-colors"
    >
      <div className="flex items-center gap-2">
        <svg
          className={`w-3 h-3 text-neutral-500 transition-transform ${isExpanded ? 'rotate-90' : ''}`}
          fill="none"
          stroke="currentColor"
          viewBox="0 0 24 24"
        >
          <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M9 5l7 7-7 7" />
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
          badge={<span className="px-1.5 py-0.5 text-xs bg-red-950 text-red-400 rounded-sm">Error</span>}
        />
        {isExpanded && (
          <div className="px-3 pb-2 space-y-2">
            <div className="bg-red-950 border border-red-900 text-red-200 px-2 py-1 rounded-sm text-xs">
              {error}
            </div>
            <Button onClick={() => { setHasFetched(false); fetchSuggestions(); }} variant="secondary" size="sm">
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
          badge={<span className="px-1.5 py-0.5 text-xs bg-green-950 text-green-400 rounded-sm">Synced</span>}
        />
        {isExpanded && (
          <div className="px-3 pb-2">
            <span className="text-xs text-neutral-500">All files synchronized.</span>
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
        <div className="px-3 pb-3 space-y-3">
          {/* Summary */}
          <div className="grid grid-cols-3 gap-2 text-center bg-neutral-950 border border-neutral-800 rounded-sm p-2">
            <div>
              <div className="text-lg font-medium text-white">{totalFiles}</div>
              <div className="text-xs text-neutral-500">Files</div>
            </div>
            <div>
              <div className="text-lg font-medium text-orange-400">{totalMirrorOperations}</div>
              <div className="text-xs text-neutral-500">Ops</div>
            </div>
            <div>
              <div className="text-lg font-medium text-green-400">{FormatBytes(totalSize)}</div>
              <div className="text-xs text-neutral-500">Size</div>
            </div>
          </div>

          {/* Server Coverage */}
          <div className="grid grid-cols-2 sm:grid-cols-3 lg:grid-cols-4 gap-2">
            {serverCoverage.map((server) => (
              <div key={server.url} className="bg-neutral-950 border border-neutral-800 rounded-sm p-2">
                <div className="flex items-center justify-between mb-1">
                  <span className="text-xs text-neutral-300 truncate">
                    {server.hostname}
                  </span>
                  <span className={`text-xs px-1 rounded-sm ${
                    server.coveragePercentage === 100
                      ? "bg-green-950 text-green-400"
                      : server.coveragePercentage >= 80
                        ? "bg-yellow-950 text-yellow-400"
                        : "bg-red-950 text-red-400"
                  }`}>
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

          {/* Mirror Action */}
          {!mirrorAllProgress ? (
            <Button
              onClick={mirrorAll}
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
                  style={{ width: `${((mirrorAllProgress.completed + mirrorAllProgress.failed) / mirrorAllProgress.total) * 100}%` }}
                />
              </div>
              <div className="flex justify-between text-xs">
                <span className="text-green-400">{mirrorAllProgress.completed} done</span>
                <span className="text-red-400">{mirrorAllProgress.failed} failed</span>
                <span className="text-neutral-500">{mirrorAllProgress.total - mirrorAllProgress.completed - mirrorAllProgress.failed} left</span>
              </div>
              {mirrorAllProgress.errors.length > 0 && (
                <div className="bg-red-950 border border-red-900 text-red-200 px-2 py-1 rounded-sm text-xs max-h-20 overflow-y-auto">
                  {mirrorAllProgress.errors.map((err, i) => (
                    <div key={i}>{err}</div>
                  ))}
                </div>
              )}
            </div>
          )}
        </div>
      )}
    </div>
  );
}
