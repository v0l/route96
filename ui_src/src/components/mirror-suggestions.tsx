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

  // Track previous servers to detect actual changes in content
  const prevServersRef = useRef<string[]>([]);
  
  // Memoize the servers array to prevent unnecessary re-renders when array contents are the same
  const memoizedServers = useMemo(() => {
    const sortedServers = [...servers].sort();
    const prevSorted = [...prevServersRef.current].sort();
    
    // Check if the arrays have the same content
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

      // Capture the servers list at the start to avoid race conditions
      const serverList = [...memoizedServers];

      if (serverList.length <= 1) {
        setLoading(false);
        return;
      }

      const fileMap: Map<string, FileMirrorSuggestion> = new Map();

      // Fetch files from each server
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
          // Continue with other servers instead of failing completely
        }
      }

      // Determine missing servers for each file using the captured server list
      for (const suggestion of fileMap.values()) {
        for (const serverUrl of serverList) {
          if (!suggestion.available_on.includes(serverUrl)) {
            suggestion.missing_from.push(serverUrl);
          }
        }
      }

      // Filter to only files that are missing from at least one server and available on at least one
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
    // Only fetch when expanded and haven't fetched yet
    if (isExpanded && memoizedServers.length > 1 && pub && login?.publicKey && !hasFetched) {
      fetchSuggestions();
      setHasFetched(true);
    }
  }, [isExpanded, memoizedServers, pub, login?.publicKey, fetchSuggestions, hasFetched]);

  async function mirrorAll() {
    if (!pub || suggestions.length === 0) return;

    // Calculate total operations needed
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

    // Mirror all files to all missing servers
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

          // Update suggestions in real-time
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

    // Keep progress visible for a moment before clearing
    setTimeout(() => {
      setMirrorAllProgress(null);
    }, 3000);
  }

  // Calculate coverage statistics
  const totalFiles = suggestions.length;
  const totalMirrorOperations = suggestions.reduce((total, suggestion) =>
    total + suggestion.missing_from.length, 0
  );
  const totalSize = suggestions.reduce((total, suggestion) => total + suggestion.size, 0);

  // Calculate coverage per server
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
    return null; // No suggestions needed for single server
  }

  // Collapsible header component
  const CollapsibleHeader = ({ title, badge }: { title: string; badge?: React.ReactNode }) => (
    <button
      onClick={() => setIsExpanded(!isExpanded)}
      className="w-full flex items-center justify-between p-4 text-left hover:bg-neutral-700/50 transition-colors"
    >
      <div className="flex items-center gap-3">
        <svg
          className={`w-4 h-4 text-neutral-400 transition-transform ${isExpanded ? 'rotate-90' : ''}`}
          fill="none"
          stroke="currentColor"
          viewBox="0 0 24 24"
        >
          <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M9 5l7 7-7 7" />
        </svg>
        <h3 className="text-lg font-semibold text-neutral-100">{title}</h3>
        {badge}
      </div>
    </button>
  );

  // Not yet fetched - show collapsed header with no badge
  if (!hasFetched && !loading) {
    return (
      <div className="bg-neutral-800 border border-neutral-700 rounded-lg shadow-sm">
        <CollapsibleHeader title="Mirror Suggestions" />
      </div>
    );
  }

  if (loading) {
    return (
      <div className="bg-neutral-800 border border-neutral-700 rounded-lg shadow-sm">
        <CollapsibleHeader title="Mirror Suggestions" />
        {isExpanded && (
          <div className="px-4 pb-4">
            <p className="text-neutral-400">Loading mirror suggestions...</p>
          </div>
        )}
      </div>
    );
  }

  if (error) {
    return (
      <div className="bg-neutral-800 border border-neutral-700 rounded-lg shadow-sm">
        <CollapsibleHeader 
          title="Mirror Suggestions" 
          badge={<span className="px-2 py-1 text-xs bg-red-900 text-red-200 rounded-full">Error</span>}
        />
        {isExpanded && (
          <div className="px-4 pb-4 space-y-4">
            <div className="bg-red-900 border border-red-700 text-red-200 px-4 py-3 rounded">
              {error}
            </div>
            <Button onClick={() => { setHasFetched(false); fetchSuggestions(); }} variant="secondary">
              Retry
            </Button>
          </div>
        )}
      </div>
    );
  }

  if (suggestions.length === 0) {
    return (
      <div className="bg-neutral-800 border border-neutral-700 rounded-lg shadow-sm">
        <CollapsibleHeader 
          title="Mirror Suggestions"
          badge={<span className="px-2 py-1 text-xs bg-green-900 text-green-200 rounded-full">Synced</span>}
        />
        {isExpanded && (
          <div className="px-4 pb-4">
            <p className="text-neutral-400">All your files are synchronized across all servers.</p>
          </div>
        )}
      </div>
    );
  }

  return (
    <div className="bg-neutral-800 border border-neutral-700 rounded-lg shadow-sm">
      <CollapsibleHeader 
        title="Mirror Coverage"
        badge={
          <span className="px-2 py-1 text-xs bg-orange-900 text-orange-200 rounded-full">
            {totalMirrorOperations} ops needed
          </span>
        }
      />
      {isExpanded && (
        <div className="px-4 pb-4">
          <div className="space-y-6">

          {/* Coverage Summary */}
          <div className="bg-neutral-700 border border-neutral-600 rounded-lg">
            <div className="p-6">
              <div className="grid grid-cols-3 gap-4 text-center">
                <div>
                  <div className="text-2xl font-bold text-neutral-100">{totalFiles}</div>
                  <div className="text-xs text-neutral-400">Files to Mirror</div>
                </div>
                <div>
                  <div className="text-2xl font-bold text-orange-400">{totalMirrorOperations}</div>
                  <div className="text-xs text-neutral-400">Operations Needed</div>
                </div>
                <div>
                  <div className="text-2xl font-bold text-green-400">{FormatBytes(totalSize)}</div>
                  <div className="text-xs text-neutral-400">Total Size</div>
                </div>
              </div>
            </div>
          </div>

          {/* Server Coverage */}
          <div className="bg-neutral-700 border border-neutral-600 rounded-lg">
            <div className="p-4 pb-3">
              <h4 className="text-sm font-semibold text-neutral-200">Coverage by Server</h4>
            </div>
            <div className="p-4">
              <div className="grid grid-cols-1 sm:grid-cols-2 lg:grid-cols-3 xl:grid-cols-4 gap-4">
                {serverCoverage.map((server) => (
                  <div key={server.url} className="bg-neutral-800 border border-neutral-600 rounded-lg">
                    <div className="p-3">
                      <div className="flex items-center justify-between mb-2">
                        <span className="text-sm font-medium truncate text-neutral-200">
                          {server.hostname}
                        </span>
                        <span
                          className={`inline-flex items-center px-2 py-1 rounded-full text-xs font-medium ${
                            server.coveragePercentage === 100
                              ? "bg-green-900 text-green-200"
                              : server.coveragePercentage >= 80
                                ? "bg-yellow-900 text-yellow-200"
                                : "bg-red-900 text-red-200"
                          }`}
                        >
                          {server.coveragePercentage}%
                        </span>
                      </div>
                      <div className="w-full bg-neutral-600 rounded-full h-2 mb-2">
                        <div 
                          className="bg-neutral-300 h-2 rounded-full transition-all duration-300"
                          style={{ width: `${server.coveragePercentage}%` }}
                        />
                      </div>
                      <div className="text-xs text-neutral-400 text-center">
                        {server.filesCount} / {server.totalFiles} files
                      </div>
                    </div>
                  </div>
                ))}
              </div>
            </div>
          </div>

          {/* Mirror All Section */}
          {!mirrorAllProgress ? (
            <div className="text-center space-y-4">
              <p className="text-neutral-400">
                {totalFiles} files need to be synchronized across your servers
              </p>
              <Button
                onClick={mirrorAll}
                disabled={totalMirrorOperations === 0}
              >
                Mirror Everything
              </Button>
            </div>
          ) : (
          <div className="space-y-4">
            {/* Progress Bar */}
            <div>
              <div className="flex justify-between text-sm mb-2">
                <span className="text-neutral-400">Progress</span>
                <span className="text-neutral-400">
                  {mirrorAllProgress.completed + mirrorAllProgress.failed} / {mirrorAllProgress.total}
                </span>
              </div>
              <div className="w-full bg-neutral-700 rounded-full h-2">
                <div 
                  className="bg-neutral-300 h-2 rounded-full transition-all duration-300"
                  style={{ width: `${((mirrorAllProgress.completed + mirrorAllProgress.failed) / mirrorAllProgress.total) * 100}%` }}
                />
              </div>
            </div>

            {/* Status Summary */}
            <div className="grid grid-cols-3 gap-4 text-center text-sm">
              <div>
                <div className="text-green-400 font-semibold">{mirrorAllProgress.completed}</div>
                <div className="text-neutral-500">Completed</div>
              </div>
              <div>
                <div className="text-red-400 font-semibold">{mirrorAllProgress.failed}</div>
                <div className="text-neutral-500">Failed</div>
              </div>
              <div>
                <div className="text-neutral-500 font-semibold">
                  {mirrorAllProgress.total - mirrorAllProgress.completed - mirrorAllProgress.failed}
                </div>
                <div className="text-neutral-500">Remaining</div>
              </div>
            </div>

            {/* Errors */}
            {mirrorAllProgress.errors.length > 0 && (
              <div className="bg-red-900 border border-red-700 text-red-200 px-4 py-3 rounded">
                <h4 className="font-semibold mb-2">Errors ({mirrorAllProgress.errors.length})</h4>
                <div className="space-y-1 max-h-32 overflow-y-auto">
                  {mirrorAllProgress.errors.map((error, index) => (
                    <div key={index} className="text-xs">{error}</div>
                  ))}
                </div>
              </div>
            )}
          </div>
          )}
          </div>
        </div>
      )}
    </div>
  );
}