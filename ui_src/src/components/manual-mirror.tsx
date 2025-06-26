import { useState } from "react";
import { Blossom } from "../upload/blossom";
import Button from "./button";
import usePublisher from "../hooks/publisher";
import useLogin from "../hooks/login";

interface ManualMirrorProps {
  servers: string[];
}

interface MirrorResult {
  server: string;
  success: boolean;
  error?: string;
  sha256?: string;
  url?: string;
}

export default function ManualMirror({ servers }: ManualMirrorProps) {
  const [url, setUrl] = useState("");
  const [selectedServers, setSelectedServers] = useState<string[]>([]);
  const [mirroring, setMirroring] = useState(false);
  const [results, setResults] = useState<MirrorResult[]>([]);

  const pub = usePublisher();
  const login = useLogin();

  const handleServerToggle = (server: string) => {
    setSelectedServers(prev => 
      prev.includes(server) 
        ? prev.filter(s => s !== server)
        : [...prev, server]
    );
  };

  const handleMirror = async () => {
    if (!pub || !url.trim() || selectedServers.length === 0) return;

    setMirroring(true);
    setResults([]);

    const newResults: MirrorResult[] = [];

    for (const server of selectedServers) {
      try {
        const blossom = new Blossom(server, pub);
        const result = await blossom.mirror(url.trim());
        
        newResults.push({
          server,
          success: true,
          sha256: result.sha256,
          url: result.url
        });
      } catch (error) {
        const errorMessage = error instanceof Error ? error.message : "Unknown error";
        newResults.push({
          server,
          success: false,
          error: errorMessage
        });
      }
    }

    setResults(newResults);
    setMirroring(false);
  };

  const isValidUrl = (str: string) => {
    try {
      new URL(str);
      return true;
    } catch {
      return false;
    }
  };

  if (!login) {
    return null;
  }

  return (
    <div className="card">
      <h2 className="text-xl font-semibold mb-4">Manual Mirror</h2>
      <p className="text-gray-400 text-sm mb-6">
        Mirror any file URL to your selected servers. The file will be downloaded and stored.
      </p>

      {/* URL Input */}
      <div className="mb-4">
        <label className="block text-sm font-medium mb-2">File URL</label>
        <input
          type="url"
          value={url}
          onChange={(e) => setUrl(e.target.value)}
          placeholder="https://example.com/path/to/file.zip"
          className="input w-full"
          disabled={mirroring}
        />
      </div>

      {/* Server Selection */}
      <div className="mb-6">
        <label className="block text-sm font-medium mb-2">Target Servers</label>
        <div className="space-y-2">
          {servers.map((server) => (
            <label key={server} className="flex items-center">
              <input
                type="checkbox"
                checked={selectedServers.includes(server)}
                onChange={() => handleServerToggle(server)}
                disabled={mirroring}
                className="mr-2"
              />
              <span className="text-sm">{new URL(server).hostname}</span>
            </label>
          ))}
        </div>
      </div>

      {/* Mirror Button */}
      <Button
        onClick={handleMirror}
        disabled={mirroring || !isValidUrl(url) || selectedServers.length === 0}
        className="btn-primary w-full mb-4"
      >
        {mirroring ? "Mirroring..." : "Mirror File"}
      </Button>

      {/* Results */}
      {results.length > 0 && (
        <div className="space-y-3">
          <h3 className="text-lg font-medium">Mirror Results</h3>
          {results.map((result, index) => (
            <div
              key={index}
              className={`p-3 rounded-lg border ${
                result.success
                  ? "bg-green-900/20 border-green-800"
                  : "bg-red-900/20 border-red-800"
              }`}
            >
              <div className="flex items-start justify-between">
                <div>
                  <div className="font-medium">
                    {result.success ? "✅" : "❌"} {new URL(result.server).hostname}
                  </div>
                  {result.success && result.sha256 && (
                    <div className="text-xs text-gray-400 mt-1 font-mono">
                      SHA256: {result.sha256}
                    </div>
                  )}
                  {result.error && (
                    <div className="text-sm text-red-400 mt-1">
                      {result.error}
                    </div>
                  )}
                </div>
              </div>
            </div>
          ))}
        </div>
      )}
    </div>
  );
}