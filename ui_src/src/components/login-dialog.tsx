import { useState } from "react";
import { Nip7Signer, PrivateKeySigner } from "@snort/system";
import { hexToBech32, bech32ToHex } from "@snort/shared";
import Button from "./button";
import { Login } from "../login";
import CreateAccountDialog from "./create-account";

type LoginMethod = "nsec" | "nip46" | "create";

export default function LoginDialog({
  onSuccess,
}: { onSuccess?: () => void } = {}) {
  const [method, setMethod] = useState<LoginMethod | null>(null);
  const [nsecInput, setNsecInput] = useState("");
  const [bunkerInput, setBunkerInput] = useState("");
  const [error, setError] = useState<string>();

  function back() {
    setMethod(null);
    setError(undefined);
  }

  async function loginNip7() {
    setError(undefined);
    try {
      const n7 = new Nip7Signer();
      const pubkey = await n7.getPubKey();
      Login.login(pubkey, "nip7");
      onSuccess?.();
    } catch (e) {
      setError(
        e instanceof Error
          ? e.message
          : "Could not connect to Nostr extension.",
      );
    }
  }

  function loginNsec() {
    setError(undefined);
    try {
      let key = nsecInput.trim();
      if (key.startsWith("nsec1")) {
        key = bech32ToHex(key);
      }
      if (!/^[0-9a-fA-F]{64}$/.test(key)) {
        setError("Invalid key — provide a 64-char hex key or nsec1… bech32.");
        return;
      }
      Login.loginPrivateKey(key);
      onSuccess?.();
    } catch (e) {
      setError(e instanceof Error ? e.message : "Invalid key.");
    }
  }

  function loginBunker() {
    setError(undefined);
    try {
      const url = bunkerInput.trim();
      if (!url.startsWith("bunker://")) {
        setError("Bunker URL must start with bunker://");
        return;
      }
      const withoutScheme = url.slice("bunker://".length);
      const remotePubkey = withoutScheme.split("?")[0];
      if (!/^[0-9a-fA-F]{64}$/.test(remotePubkey)) {
        setError("Could not parse remote pubkey from bunker URL.");
        return;
      }
      const localKey = PrivateKeySigner.random().privateKey;
      Login.loginBunker(url, localKey, remotePubkey);
      onSuccess?.();
    } catch (e) {
      setError(e instanceof Error ? e.message : "Invalid bunker URL.");
    }
  }

  // Derive npub preview while the user types an nsec/hex key
  const nsecPreview = (() => {
    try {
      let key = nsecInput.trim();
      if (key.startsWith("nsec1")) key = bech32ToHex(key);
      if (/^[0-9a-fA-F]{64}$/.test(key)) {
        const signer = new PrivateKeySigner(key);
        return hexToBech32("npub", signer.getPubKey()).slice(0, 20) + "…";
      }
    } catch {
      // ignore
    }
    return null;
  })();

  return (
    <div className="max-w-md mx-auto bg-neutral-900 border border-neutral-800 rounded-sm p-6 space-y-4">
      <div>
        <h2 className="text-lg font-medium text-white">
          Welcome to {window.location.hostname}
        </h2>
        <p className="text-neutral-400 text-sm mt-1">
          Sign in with your Nostr identity to start uploading files.
        </p>
      </div>

      {method === null && (
        <div className="space-y-2">
          <button
            className="w-full text-left px-3 py-2.5 rounded-sm border border-neutral-700 bg-neutral-800 hover:border-neutral-500 transition-colors"
            onClick={loginNip7}
          >
            <div className="text-sm font-medium text-white">
              Browser extension
            </div>
            <div className="text-xs text-neutral-400 mt-0.5">
              nos2x, Alby, Nostr Connect, or any NIP-07 extension
            </div>
          </button>

          <button
            className="w-full text-left px-3 py-2.5 rounded-sm border border-neutral-700 bg-neutral-800 hover:border-neutral-500 transition-colors"
            onClick={() => {
              setError(undefined);
              setMethod("nsec");
            }}
          >
            <div className="text-sm font-medium text-white">Private key</div>
            <div className="text-xs text-neutral-400 mt-0.5">
              Paste your nsec1… or hex private key directly
            </div>
          </button>

          <button
            className="w-full text-left px-3 py-2.5 rounded-sm border border-neutral-700 bg-neutral-800 hover:border-neutral-500 transition-colors"
            onClick={() => {
              setError(undefined);
              setMethod("nip46");
            }}
          >
            <div className="text-sm font-medium text-white">Remote signer</div>
            <div className="text-xs text-neutral-400 mt-0.5">
              Connect via a NIP-46 bunker:// URL
            </div>
          </button>

          <div className="border-t border-neutral-800 pt-2">
            <button
              className="w-full text-left px-3 py-2.5 rounded-sm border border-neutral-700 bg-neutral-800 hover:border-neutral-500 transition-colors"
              onClick={() => {
                setError(undefined);
                setMethod("create");
              }}
            >
              <div className="text-sm font-medium text-white">
                Create account
              </div>
              <div className="text-xs text-neutral-400 mt-0.5">
                Generate a new Nostr keypair and set up your profile
              </div>
            </button>
          </div>
        </div>
      )}

      {method === "nsec" && (
        <div className="space-y-3">
          <button
            className="text-xs text-neutral-500 hover:text-neutral-300 transition-colors"
            onClick={back}
          >
            ← Back
          </button>
          <div>
            <label className="block text-xs text-neutral-400 mb-1">
              Private key
            </label>
            <input
              type="password"
              className="w-full bg-neutral-800 border border-neutral-700 rounded-sm px-3 py-2 text-sm text-white placeholder-neutral-600 focus:outline-none focus:border-neutral-500"
              placeholder="nsec1… or 64-char hex"
              value={nsecInput}
              onChange={(e) => {
                setNsecInput(e.target.value);
                setError(undefined);
              }}
              autoFocus
            />
            {nsecPreview && (
              <p className="text-xs text-neutral-500 mt-1 font-mono">
                {nsecPreview}
              </p>
            )}
          </div>
          <Button
            className="w-full"
            disabled={!nsecInput.trim()}
            onClick={loginNsec}
          >
            Sign in
          </Button>
        </div>
      )}

      {method === "nip46" && (
        <div className="space-y-3">
          <button
            className="text-xs text-neutral-500 hover:text-neutral-300 transition-colors"
            onClick={back}
          >
            ← Back
          </button>
          <div>
            <label className="block text-xs text-neutral-400 mb-1">
              Bunker URL
            </label>
            <input
              type="text"
              className="w-full bg-neutral-800 border border-neutral-700 rounded-sm px-3 py-2 text-sm text-white placeholder-neutral-600 focus:outline-none focus:border-neutral-500"
              placeholder="bunker://…"
              value={bunkerInput}
              onChange={(e) => {
                setBunkerInput(e.target.value);
                setError(undefined);
              }}
              autoFocus
            />
          </div>
          <Button
            className="w-full"
            disabled={!bunkerInput.trim()}
            onClick={loginBunker}
          >
            Connect
          </Button>
        </div>
      )}

      {method === "create" && (
        <CreateAccountDialog onBack={back} onSuccess={onSuccess} />
      )}

      {method !== "create" && error && (
        <p className="text-red-400 text-xs">{error}</p>
      )}
    </div>
  );
}
