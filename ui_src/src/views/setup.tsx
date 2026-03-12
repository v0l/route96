import { useState } from "react";
import Button from "../components/button";
import LoginDialog from "../components/login-dialog";
import useLogin from "../hooks/login";
import usePublisher from "../hooks/publisher";
import { Route96 } from "../upload/admin";

export default function Setup() {
  const login = useLogin();
  const pub = usePublisher();
  const [publicUrl, setPublicUrl] = useState(
    `${window.location.protocol}//${window.location.host}`,
  );
  const [maxUploadMb, setMaxUploadMb] = useState(100);
  const [saving, setSaving] = useState(false);
  const [done, setDone] = useState(false);
  const [error, setError] = useState<string>();

  const baseUrl =
    import.meta.env.VITE_API_URL ||
    `${window.location.protocol}//${window.location.host}`;

  async function handleSubmit(e: React.FormEvent) {
    e.preventDefault();
    if (!pub) return;
    setError(undefined);
    setSaving(true);
    try {
      const r96 = new Route96(baseUrl, pub);
      await r96.postSetup({
        public_url: publicUrl,
        max_upload_bytes: maxUploadMb * 1024 * 1024,
      });
      setDone(true);
      // Reload so the header re-fetches /admin/self and clears the banner.
      window.location.href = "/";
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    } finally {
      setSaving(false);
    }
  }

  return (
    <div className="max-w-xl mx-auto mt-16 px-4">
      <div className="bg-neutral-900 border border-neutral-800 rounded-lg p-8">
        <h1 className="text-2xl font-bold text-white mb-2">
          Welcome to route96
        </h1>
        <p className="text-neutral-400 mb-8 text-sm">
          Complete this one-time setup to configure your server. The Nostr key
          you sign in with will become the administrator account.
        </p>

        {!login ? (
          <LoginDialog />
        ) : (
          <form onSubmit={handleSubmit} className="space-y-6">
            <div>
              <label className="block text-sm font-medium text-neutral-300 mb-1">
                Public URL
              </label>
              <p className="text-xs text-neutral-500 mb-2">
                The public-facing base URL for this server. Used to build file
                download links.
              </p>
              <input
                type="url"
                required
                value={publicUrl}
                onChange={(e) => setPublicUrl(e.target.value)}
                placeholder="https://cdn.example.com"
                className="w-full bg-neutral-800 border border-neutral-700 rounded px-3 py-2 text-white text-sm focus:outline-none focus:border-neutral-500 placeholder-neutral-600"
              />
            </div>

            <div>
              <label className="block text-sm font-medium text-neutral-300 mb-1">
                Max upload size (MB)
              </label>
              <p className="text-xs text-neutral-500 mb-2">
                Maximum file size allowed per upload.
              </p>
              <input
                type="number"
                required
                min={1}
                value={maxUploadMb}
                onChange={(e) => setMaxUploadMb(Number(e.target.value))}
                className="w-full bg-neutral-800 border border-neutral-700 rounded px-3 py-2 text-white text-sm focus:outline-none focus:border-neutral-500"
              />
            </div>

            {error && (
              <div className="bg-red-950 border border-red-800 rounded px-3 py-2 text-red-300 text-sm">
                {error}
              </div>
            )}

            <Button type="submit" disabled={saving || done} className="w-full">
              {saving ? "Saving..." : "Complete Setup"}
            </Button>
          </form>
        )}
      </div>
    </div>
  );
}
