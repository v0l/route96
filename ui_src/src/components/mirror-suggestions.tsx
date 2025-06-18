import { useState, useEffect } from "react";
import { FileMirrorSuggestion, Blossom } from "../upload/blossom";
import { FormatBytes } from "../const";
import Button from "./button";
import usePublisher from "../hooks/publisher";

interface MirrorSuggestionsProps {
  servers: string[];
  currentServer: string;
}

export default function MirrorSuggestions({ servers, currentServer }: MirrorSuggestionsProps) {
  const [suggestions, setSuggestions] = useState<FileMirrorSuggestion[]>([]);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string>();
  const [mirroring, setMirroring] = useState<Set<string>>(new Set());

  const pub = usePublisher();

  useEffect(() => {
    if (servers.length > 1 && pub) {
      fetchSuggestions();
    }
  }, [servers, pub]);

  async function fetchSuggestions() {
    if (!pub) return;
    
    try {
      setLoading(true);
      setError(undefined);
      
      const blossom = new Blossom(currentServer, pub);
      const result = await blossom.getMirrorSuggestions(servers);
      setSuggestions(result.suggestions);
    } catch (e) {
      if (e instanceof Error) {
        setError(e.message);
      } else {
        setError("Failed to fetch mirror suggestions");
      }
    } finally {
      setLoading(false);
    }
  }

  async function mirrorFile(suggestion: FileMirrorSuggestion, targetServer: string) {
    if (!pub) return;
    
    const mirrorKey = `${suggestion.sha256}-${targetServer}`;
    setMirroring(prev => new Set(prev.add(mirrorKey)));
    
    try {
      const blossom = new Blossom(targetServer, pub);
      await blossom.mirror(suggestion.url);
      
      // Update suggestions by removing this server from missing_from
      setSuggestions(prev => 
        prev.map(s => 
          s.sha256 === suggestion.sha256 
            ? {
                ...s,
                available_on: [...s.available_on, targetServer],
                missing_from: s.missing_from.filter(server => server !== targetServer)
              }
            : s
        ).filter(s => s.missing_from.length > 0) // Remove suggestions with no missing servers
      );
    } catch (e) {
      if (e instanceof Error) {
        setError(`Failed to mirror file: ${e.message}`);
      } else {
        setError("Failed to mirror file");
      }
    } finally {
      setMirroring(prev => {
        const newSet = new Set(prev);
        newSet.delete(mirrorKey);
        return newSet;
      });
    }
  }

  if (servers.length <= 1) {
    return null; // No suggestions needed for single server
  }

  if (loading) {
    return (
      <div className="card">
        <h3 className="text-lg font-semibold mb-4">Mirror Suggestions</h3>
        <p className="text-gray-400">Loading mirror suggestions...</p>
      </div>
    );
  }

  if (error) {
    return (
      <div className="card">
        <h3 className="text-lg font-semibold mb-4">Mirror Suggestions</h3>
        <div className="bg-red-900/20 border border-red-800 text-red-400 px-4 py-3 rounded-lg mb-4">
          {error}
        </div>
        <Button onClick={fetchSuggestions} className="btn-secondary">
          Retry
        </Button>
      </div>
    );
  }

  if (suggestions.length === 0) {
    return (
      <div className="card">
        <h3 className="text-lg font-semibold mb-4">Mirror Suggestions</h3>
        <p className="text-gray-400">All your files are synchronized across all servers.</p>
      </div>
    );
  }

  return (
    <div className="card">
      <h3 className="text-lg font-semibold mb-4">Mirror Suggestions</h3>
      <p className="text-gray-400 mb-6">
        The following files are missing from some of your servers and can be mirrored:
      </p>
      
      <div className="space-y-4">
        {suggestions.map((suggestion) => (
          <div key={suggestion.sha256} className="bg-gray-800 border border-gray-700 rounded-lg p-4">
            <div className="flex items-start justify-between mb-3">
              <div className="flex-1">
                <p className="text-sm font-medium text-gray-300 mb-1">
                  File: {suggestion.sha256.substring(0, 16)}...
                </p>
                <p className="text-xs text-gray-400">
                  Size: {FormatBytes(suggestion.size)}
                  {suggestion.mime_type && ` • Type: ${suggestion.mime_type}`}
                </p>
              </div>
            </div>
            
            <div className="space-y-2">
              <div>
                <p className="text-xs text-green-400 mb-1">Available on:</p>
                <div className="flex flex-wrap gap-1">
                  {suggestion.available_on.map((server) => (
                    <span key={server} className="text-xs bg-green-900/30 text-green-300 px-2 py-1 rounded">
                      {new URL(server).hostname}
                    </span>
                  ))}
                </div>
              </div>
              
              <div>
                <p className="text-xs text-red-400 mb-1">Missing from:</p>
                <div className="flex flex-wrap gap-2">
                  {suggestion.missing_from.map((server) => {
                    const mirrorKey = `${suggestion.sha256}-${server}`;
                    const isMirroring = mirroring.has(mirrorKey);
                    
                    return (
                      <div key={server} className="flex items-center gap-2">
                        <span className="text-xs bg-red-900/30 text-red-300 px-2 py-1 rounded">
                          {new URL(server).hostname}
                        </span>
                        <Button
                          onClick={() => mirrorFile(suggestion, server)}
                          disabled={isMirroring}
                          className="btn-primary text-xs py-1 px-2"
                        >
                          {isMirroring ? "Mirroring..." : "Mirror"}
                        </Button>
                      </div>
                    );
                  })}
                </div>
              </div>
            </div>
          </div>
        ))}
      </div>
    </div>
  );
}