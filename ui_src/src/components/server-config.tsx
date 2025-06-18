import { useState, useEffect } from "react";
import Button from "./button";
import useLogin from "../hooks/login";
import { useBlossomServers } from "../hooks/use-blossom-servers";

interface ServerConfigProps {
  onServersChange: (servers: string[]) => void;
}

export default function ServerConfig({ onServersChange }: ServerConfigProps) {
  const login = useLogin();
  const nostrServers = useBlossomServers(); // Servers from nostr event kind 10063
  const [manualServers, setManualServers] = useState<string[]>([]);
  const [newServer, setNewServer] = useState("");
  const [error, setError] = useState<string>();

  // Combine nostr servers with manually added servers
  const allServers = [...nostrServers, ...manualServers];

  useEffect(() => {
    onServersChange(allServers);
  }, [allServers, onServersChange]);

  function addServer() {
    if (!newServer.trim()) {
      setError("Please enter a server URL");
      return;
    }

    try {
      const url = new URL(newServer.trim());
      const serverUrl = url.toString().replace(/\/$/, ""); // Remove trailing slash
      
      if (allServers.includes(serverUrl)) {
        setError("Server already added");
        return;
      }

      const updatedServers = [...manualServers, serverUrl];
      setManualServers(updatedServers);
      setNewServer("");
      setError(undefined);
    } catch (e) {
      setError("Please enter a valid URL");
    }
  }

  function removeServer(serverToRemove: string) {
    // Only allow removing manually added servers
    if (nostrServers.includes(serverToRemove)) {
      setError("Cannot remove servers from nostr profile. Update your kind 10063 event to remove them.");
      return;
    }
    
    const updatedServers = manualServers.filter(s => s !== serverToRemove);
    setManualServers(updatedServers);
    setError(undefined);
  }

  function addCurrentServer() {
    const currentUrl = `${window.location.protocol}//${window.location.host}`;
    if (!allServers.includes(currentUrl)) {
      const updatedServers = [...manualServers, currentUrl];
      setManualServers(updatedServers);
    }
  }

  return (
    <div className="card">
      <h3 className="text-lg font-semibold mb-4">Blossom Servers</h3>
      <p className="text-gray-400 mb-4">
        Configure your blossom servers to get mirror suggestions across them. 
        Servers are loaded from your nostr profile event (kind 10063).
      </p>

      {!login?.pubkey && (
        <div className="bg-yellow-900/20 border border-yellow-800 text-yellow-400 px-4 py-3 rounded-lg mb-4">
          Please log in to configure your server list.
        </div>
      )}

      {error && (
        <div className="bg-red-900/20 border border-red-800 text-red-400 px-4 py-3 rounded-lg mb-4">
          {error}
        </div>
      )}

      <div className="space-y-4">
        {/* Current servers */}
        {allServers.length > 0 && (
          <div>
            <h4 className="text-sm font-medium text-gray-300 mb-2">Configured Servers:</h4>
            <div className="space-y-2">
              {allServers.map((server) => {
                const isFromNostr = nostrServers.includes(server);
                return (
                  <div key={server} className="flex items-center justify-between bg-gray-800 px-3 py-2 rounded">
                    <div className="flex items-center gap-2">
                      <span className="text-sm text-gray-300">{server}</span>
                      {isFromNostr && (
                        <span className="text-xs bg-blue-900/50 text-blue-300 px-2 py-1 rounded">
                          from nostr
                        </span>
                      )}
                    </div>
                    {!isFromNostr && (
                      <Button
                        onClick={() => removeServer(server)}
                        className="btn-secondary text-xs py-1 px-2"
                      >
                        Remove
                      </Button>
                    )}
                  </div>
                );
              })}
            </div>
          </div>
        )}

        {login?.pubkey && allServers.length === 0 && (
          <div className="text-gray-400 text-sm">
            No servers configured. Add servers below to enable mirror suggestions, or create a kind 10063 nostr event with server tags.
          </div>
        )}

        {/* Add server form */}
        {login?.pubkey && (
          <div>
            <h4 className="text-sm font-medium text-gray-300 mb-2">Add Server:</h4>
            <div className="flex gap-2">
              <input
                type="url"
                value={newServer}
                onChange={(e) => setNewServer(e.target.value)}
                placeholder="https://example.com"
                className="flex-1 bg-gray-800 border border-gray-700 rounded px-3 py-2 text-sm text-gray-100 placeholder-gray-500 focus:outline-none focus:ring-2 focus:ring-blue-500"
                onKeyPress={(e) => e.key === "Enter" && addServer()}
              />
              <Button onClick={addServer} className="btn-primary">
                Add
              </Button>
            </div>
          </div>
        )}

        {/* Quick add current server */}
        {login?.pubkey && !allServers.includes(`${window.location.protocol}//${window.location.host}`) && (
          <div className="pt-2 border-t border-gray-700">
            <Button onClick={addCurrentServer} className="btn-secondary text-sm">
              Add Current Server ({window.location.host})
            </Button>
          </div>
        )}
      </div>
    </div>
  );
}