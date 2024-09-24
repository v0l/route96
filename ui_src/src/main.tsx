import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import App from "./App.tsx";
import "./index.css";
import { NostrSystem } from "@snort/system";
import { SnortContext } from "@snort/system-react";

const system = new NostrSystem({});
[
  "wss://nos.lol/",
  "wss://relay.damus.io/",
  "wss://relay.nostr.band/",
  "wss://relay.snort.social/",
].map((a) => system.ConnectToRelay(a, { read: true, write: true }));

createRoot(document.getElementById("root")!).render(
  <StrictMode>
    <SnortContext.Provider value={system}>
      <App />
    </SnortContext.Provider>
  </StrictMode>,
);
