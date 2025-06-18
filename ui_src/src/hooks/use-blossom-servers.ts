import { useState, useEffect } from "react";
import useLogin from "./login";

export function useBlossomServers(): string[] {
  const login = useLogin();
  const [servers, setServers] = useState<string[]>([]);

  useEffect(() => {
    if (!login?.pubkey) {
      setServers([]);
      return;
    }

    // TODO: Actually implement nostr event kind 10063 querying
    // For now, return empty array - user can manually add servers
    // The API for @snort/system needs to be clarified for proper implementation
    setServers([]);
    
  }, [login?.pubkey]);

  return servers;
}