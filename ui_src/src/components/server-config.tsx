import { useState, useEffect } from "react";
import Button from "./button";

interface ServerConfigProps {
  onServersChange: (servers: string[]) => void;
}

const STORAGE_KEY = "blossom-servers";

export default function ServerConfig({ onServersChange }: ServerConfigProps) {
  const [servers, setServers] = useState<string[]>([]);
  const [newServer, setNewServer] = useState("");
  const [error, setError] = useState<string>();

  useEffect(() => {
    // Load servers from localStorage
    try {
      const stored = localStorage.getItem(STORAGE_KEY);
      if (stored) {
        const parsedServers = JSON.parse(stored);
        if (Array.isArray(parsedServers)) {
          setServers(parsedServers);
          onServersChange(parsedServers);
        }
      }
    } catch (e) {
      console.error("Failed to load servers from localStorage:", e);
    }
  }, [onServersChange]);

  function saveServers(newServers: string[]) {
    try {
      localStorage.setItem(STORAGE_KEY, JSON.stringify(newServers));
      setServers(newServers);
      onServersChange(newServers);
    } catch (e) {
      setError("Failed to save server configuration");
      console.error("Failed to save servers to localStorage:", e);
    }
  }

  function addServer() {
    if (!newServer.trim()) {
      setError("Please enter a server URL");
      return;
    }

    try {
      const url = new URL(newServer.trim());
      const serverUrl = url.toString().replace(/\/$/, ""); // Remove trailing slash
      
      if (servers.includes(serverUrl)) {
        setError("Server already added");
        return;
      }

      const updatedServers = [...servers, serverUrl];
      saveServers(updatedServers);
      setNewServer("");
      setError(undefined);
    } catch (e) {
      setError("Please enter a valid URL");
    }
  }

  function removeServer(serverToRemove: string) {
    const updatedServers = servers.filter(s => s !== serverToRemove);
    saveServers(updatedServers);
  }

  function addCurrentServer() {
    const currentUrl = `${window.location.protocol}//${window.location.host}`;
    if (!servers.includes(currentUrl)) {
      const updatedServers = [...servers, currentUrl];
      saveServers(updatedServers);
    }
  }

  return (
    <div className="card">
      <h3 className="text-lg font-semibold mb-4">Blossom Servers</h3>
      <p className="text-gray-400 mb-4">
        Configure your blossom servers to get mirror suggestions across them.
      </p>

      {error && (
        <div className="bg-red-900/20 border border-red-800 text-red-400 px-4 py-3 rounded-lg mb-4">
          {error}
        </div>
      )}

      <div className="space-y-4">
        {/* Current servers */}
        {servers.length > 0 && (
          <div>
            <h4 className="text-sm font-medium text-gray-300 mb-2">Configured Servers:</h4>
            <div className="space-y-2">
              {servers.map((server) => (
                <div key={server} className="flex items-center justify-between bg-gray-800 px-3 py-2 rounded">
                  <span className="text-sm text-gray-300">{server}</span>
                  <Button
                    onClick={() => removeServer(server)}
                    className="btn-secondary text-xs py-1 px-2"
                  >
                    Remove
                  </Button>
                </div>
              ))}
            </div>
          </div>
        )}

        {/* Add server form */}
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

        {/* Quick add current server */}
        {!servers.includes(`${window.location.protocol}//${window.location.host}`) && (
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