import { useMemo } from "react";
import useLogin from "./login";

const DefaultMediaServers = ["https://cdn.satellite.earth/", "https://cdn.self.hosted/"];

export function useBlossomServers() {
  const login = useLogin();

  return useMemo(() => {
    // For now, just return default servers
    // TODO: Implement proper nostr event kind 10063 querying when system supports it
    const servers = DefaultMediaServers;

    return {
      servers,
      addServer: async (serverUrl: string) => {
        // TODO: Implement adding server to event kind 10063
        console.log("Adding server not implemented yet:", serverUrl);
      },
      removeServer: async (serverUrl: string) => {
        // TODO: Implement removing server from event kind 10063
        console.log("Removing server not implemented yet:", serverUrl);
      },
    };
  }, [login?.pubkey]);
}