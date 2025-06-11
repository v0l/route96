import { Nip7Signer, NostrLink } from "@snort/system";
import { Link, useLocation } from "react-router-dom";
import { useEffect, useState } from "react";
import Button from "../components/button";
import Profile from "../components/profile";
import useLogin from "../hooks/login";
import usePublisher from "../hooks/publisher";
import { Login } from "../login";
import { AdminSelf, Route96 } from "../upload/admin";

export default function Header() {
  const login = useLogin();
  const pub = usePublisher();
  const location = useLocation();
  const [self, setSelf] = useState<AdminSelf>();

  const url =
    import.meta.env.VITE_API_URL || `${window.location.protocol}//${window.location.host}`;

  async function tryLogin() {
    try {
      const n7 = new Nip7Signer();
      const pubkey = await n7.getPubKey();
      Login.login({
        type: "nip7",
        pubkey,
      });
    } catch {
      //ignore
    }
  }

  useEffect(() => {
    if (pub && !self) {
      const r96 = new Route96(url, pub);
      r96.getSelf().then((v) => setSelf(v.data)).catch(() => {});
    }
  }, [pub, self, url]);

  return (
    <header className="border-b border-gray-700 bg-gray-800">
      <div className="flex justify-between items-center py-4">
        <div className="flex items-center space-x-8">
          <Link to="/">
            <div className="text-2xl font-bold text-gray-100 hover:text-blue-400 transition-colors">
              route96
            </div>
          </Link>
          
          <nav className="flex space-x-6">
            <Link 
              to="/" 
              className={`text-sm font-medium transition-colors ${
                location.pathname === "/" 
                  ? "text-blue-400 border-b-2 border-blue-400 pb-1" 
                  : "text-gray-300 hover:text-gray-100"
              }`}
            >
              Upload
            </Link>
            
            {self?.is_admin && (
              <Link 
                to="/admin" 
                className={`text-sm font-medium transition-colors ${
                  location.pathname === "/admin" 
                    ? "text-blue-400 border-b-2 border-blue-400 pb-1" 
                    : "text-gray-300 hover:text-gray-100"
                }`}
              >
                Admin
              </Link>
            )}
          </nav>
        </div>

        <div className="flex items-center space-x-4">
          {login ? (
            <div className="flex items-center space-x-3">
              <Profile link={NostrLink.publicKey(login.pubkey)} />
              <Button 
                onClick={() => Login.logout()}
                className="btn-secondary text-sm"
              >
                Logout
              </Button>
            </div>
          ) : (
            <Button onClick={tryLogin} className="btn-primary">
              Login
            </Button>
          )}
        </div>
      </div>
    </header>
  );
}
