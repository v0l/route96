if (!window.crypto.randomUUID) {
  window.crypto.randomUUID = () => {
    if (crypto && crypto.getRandomValues) {
      const arr = new Uint8Array(16);
      crypto.getRandomValues(arr);
      arr[6] = (arr[6] & 0x0f) | 0x40;
      arr[8] = (arr[8] & 0x3f) | 0x80;
      return Array.from(arr, (b) => b.toString(16).padStart(2, "0")).join("").replace(/^(.{8})(.{4})(.{4})(.{4})(.{12})$/, "$1-$2-$3-$4-$5");
    }
    const arr = new Uint8Array(16);
    for (let i = 0; i < 16; i++) arr[i] = Math.floor(Math.random() * 256);
    arr[6] = (arr[6] & 0x0f) | 0x40;
    arr[8] = (arr[8] & 0x3f) | 0x80;
    return Array.from(arr, (b) => b.toString(16).padStart(2, "0")).join("").replace(/^(.{8})(.{4})(.{4})(.{4})(.{12})$/, "$1-$2-$3-$4-$5");
  };
}

import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import App from "./App.tsx";
import "./index.css";
import { NostrSystem } from "@snort/system";
import { SnortContext } from "@snort/system-react";
import { createBrowserRouter, RouterProvider } from "react-router-dom";
import Upload from "./views/upload.tsx";
import Admin from "./views/admin.tsx";
import UserScope from "./views/user-scope.tsx";
import Setup from "./views/setup.tsx";

const system = new NostrSystem({});
[
  "wss://nos.lol/",
  "wss://relay.damus.io/",
  "wss://relay.nostr.band/",
  "wss://relay.snort.social/",
].map((a) => system.ConnectToRelay(a, { read: true, write: true }));

const routes = createBrowserRouter([
  {
    path: "",
    element: <App />,
    children: [
      {
        path: "/",
        element: <Upload />,
      },
      {
        path: "/setup",
        element: <Setup />,
      },
      {
        path: "/admin",
        element: <Admin />,
      },
      {
        path: "/admin/user/:pubkey",
        element: <UserScope />,
      },
    ],
  },
]);

createRoot(document.getElementById("root")!).render(
  <StrictMode>
    <SnortContext.Provider value={system}>
      <RouterProvider router={routes} />
    </SnortContext.Provider>
  </StrictMode>,
);
