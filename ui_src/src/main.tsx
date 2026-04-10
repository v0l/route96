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
import Tos from "./views/tos.tsx";
import { v4 as uuid } from "uuid";

// Polyfill to fix 
if (!window.crypto.randomUUID) {
  //@ts-ignore
  window.crypto.randomUUID = () => {
    return uuid();
  };
}

const system = new NostrSystem({});
[
  "wss://nos.lol/",
  "wss://relay.damus.io/",
  "wss://relay.snort.social/",
  "wss://relay.primal.net/"
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
      {
        path: "/tos",
        element: <Tos />,
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
