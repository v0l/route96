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
        element: <Upload />
      },
      {
        path: "/admin",
        element: <Admin />
      },
      {
        path: "/admin/user/:pubkey",
        element: <UserScope />
      }
    ]
  }
]);


createRoot(document.getElementById("root")!).render(
  <StrictMode>
    <SnortContext.Provider value={system}>
      <RouterProvider router={routes} />
    </SnortContext.Provider>
  </StrictMode>,
);
