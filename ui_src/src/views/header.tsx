import { NostrLink } from "@snort/system";
import { Link, useLocation } from "react-router-dom";
import { useEffect, useState } from "react";
import Button from "../components/button";
import Profile from "../components/profile";
import LoginDialog from "../components/login-dialog";
import useLogin from "../hooks/login";
import usePublisher from "../hooks/publisher";
import { Login } from "../login";
import { AdminSelf, Route96 } from "../upload/admin";

export default function Header() {
  const login = useLogin();
  const pub = usePublisher();
  const location = useLocation();
  const [self, setSelf] = useState<AdminSelf>();
  const [showLogin, setShowLogin] = useState(false);

  const url =
    import.meta.env.VITE_API_URL ||
    `${window.location.protocol}//${window.location.host}`;

  useEffect(() => {
    if (pub && self === undefined) {
      const r96 = new Route96(url, pub);
      r96
        .getSelf()
        .then((v) => setSelf(v.data))
        .catch(() => {});
    }
  }, [pub, self, url]);

  return (
    <header className="border-b border-neutral-800 bg-neutral-950">
      <div className="px-4 flex justify-between items-center py-2">
        <div className="flex items-center gap-6">
          <Link to="/">
            <div className="text-lg font-bold text-white hover:text-neutral-300 transition-colors">
              route96
            </div>
          </Link>

          <nav className="flex gap-4">
            <Link
              to="/"
              className={`text-sm transition-colors ${
                location.pathname === "/"
                  ? "text-white"
                  : "text-neutral-500 hover:text-white"
              }`}
            >
              Upload
            </Link>

            {self?.is_admin && (
              <Link
                to="/admin"
                className={`text-sm transition-colors ${
                  location.pathname === "/admin"
                    ? "text-white"
                    : "text-neutral-500 hover:text-white"
                }`}
              >
                Admin
              </Link>
            )}
          </nav>
        </div>

        <div className="flex items-center gap-3">
          <a
            href="/docs.md"
            target="_blank"
            className="text-sm text-neutral-500 hover:text-white transition-colors flex items-center gap-1"
          >
            <svg
              xmlns="http://www.w3.org/2000/svg"
              width="12"
              height="12"
              viewBox="0 0 24 24"
              fill="none"
              stroke="currentColor"
              strokeWidth="2"
              strokeLinecap="round"
              strokeLinejoin="round"
            >
              <path d="M10 13a5 5 0 0 0 7.54.54l3-3a5 5 0 0 0-7.07-7.07l-1.72 1.71" />
              <path d="M14 11a5 5 0 0 0-7.54-.54l-3 3a5 5 0 0 0 7.07 7.07l1.71-1.71" />
            </svg>
            API Docs
          </a>
          <a
            href="/SKILL.md"
            target="_blank"
            className="text-sm text-neutral-500 hover:text-white transition-colors flex items-center gap-1"
          >
            <svg
              xmlns="http://www.w3.org/2000/svg"
              width="12"
              height="12"
              viewBox="0 0 24 24"
              fill="none"
              stroke="currentColor"
              strokeWidth="2"
              strokeLinecap="round"
              strokeLinejoin="round"
            >
              <path d="M10 13a5 5 0 0 0 7.54.54l3-3a5 5 0 0 0-7.07-7.07l-1.72 1.71" />
              <path d="M14 11a5 5 0 0 0-7.54-.54l-3 3a5 5 0 0 0 7.07 7.07l1.71-1.71" />
            </svg>
            SKILL.md
          </a>
          {login ? (
            <div className="flex items-center gap-2">
              <Profile link={NostrLink.publicKey(login.publicKey)} />
              <Button
                onClick={() => Login.logout()}
                variant="secondary"
                size="sm"
              >
                Logout
              </Button>
            </div>
          ) : (
            <Button onClick={() => setShowLogin(true)} size="sm">
              Login
            </Button>
          )}
        </div>
      </div>
      {showLogin && (
        <div
          className="fixed inset-0 z-50 flex items-center justify-center bg-black/70 px-4"
          onClick={(e) => {
            if (e.target === e.currentTarget) setShowLogin(false);
          }}
        >
          <div className="w-full max-w-md relative">
            <button
              className="absolute -top-8 right-0 text-xs text-neutral-500 hover:text-neutral-300 transition-colors"
              onClick={() => setShowLogin(false)}
            >
              ✕ Close
            </button>
            <LoginDialog onSuccess={() => setShowLogin(false)} />
          </div>
        </div>
      )}
    </header>
  );
}
