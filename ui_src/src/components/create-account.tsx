import { useContext, useRef, useState } from "react";
import { PrivateKeySigner, EventKind, EventPublisher } from "@snort/system";
import { hexToBech32 } from "@snort/shared";
import { SnortContext } from "@snort/system-react";
import Button from "./button";
import { Login } from "../login";
import { Blossom } from "../upload/blossom";
import { ServerUrl } from "../const";

type Step = "setup" | "save-key" | "publishing";

export default function CreateAccountDialog({
  onBack,
  onSuccess,
}: {
  onBack: () => void;
  onSuccess?: () => void;
}) {
  const system = useContext(SnortContext);

  const [step, setStep] = useState<Step>("setup");
  const [name, setName] = useState("");
  const [about, setAbout] = useState("");
  const [avatarFile, setAvatarFile] = useState<File | null>(null);
  const [avatarPreview, setAvatarPreview] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [statusMsg, setStatusMsg] = useState("");
  const [signer, setSigner] = useState<PrivateKeySigner | null>(null);
  const [copied, setCopied] = useState(false);
  const fileInputRef = useRef<HTMLInputElement>(null);

  function pickAvatar(e: React.ChangeEvent<HTMLInputElement>) {
    const f = e.target.files?.[0];
    if (!f) return;
    setAvatarFile(f);
    setAvatarPreview(URL.createObjectURL(f));
  }

  function generateKey() {
    const s = PrivateKeySigner.random();
    setSigner(s);
    setStep("save-key");
  }

  async function copyNsec() {
    if (!signer) return;
    await navigator.clipboard.writeText(hexToBech32("nsec", signer.privateKey));
    setCopied(true);
    setTimeout(() => setCopied(false), 2000);
  }

  async function publish() {
    if (!signer) return;
    setError(null);
    setStep("publishing");

    try {
      const pubkeyHex = signer.getPubKey();
      const publisher = new EventPublisher(signer, pubkeyHex);

      // 1. Optionally upload avatar via Blossom
      let picture: string | undefined;
      if (avatarFile) {
        setStatusMsg("Uploading avatar…");
        const blossom = new Blossom(ServerUrl, publisher);
        const blob = await blossom.upload(avatarFile);
        picture = blob.url;
      }

      // 2. Build and broadcast kind-0
      setStatusMsg("Publishing profile…");
      const metadata: Record<string, string> = {};
      if (name.trim()) metadata.name = name.trim();
      if (about.trim()) metadata.about = about.trim();
      if (picture) metadata.picture = picture;

      const profileEvent = await publisher.generic((eb) =>
        eb.kind(EventKind.SetMetadata).content(JSON.stringify(metadata)),
      );
      await system.BroadcastEvent(profileEvent);

      // 3. Publish kind-10063 blossom server list
      const serverListEvent = await publisher.generic((eb) =>
        eb.kind(10_063 as EventKind).tag(["server", ServerUrl]),
      );
      await system.BroadcastEvent(serverListEvent);

      // 4. Log in
      Login.loginPrivateKey(signer.privateKey);
      onSuccess?.();
    } catch (e) {
      setError(e instanceof Error ? e.message : "Something went wrong.");
      setStep("save-key");
    }
  }

  const nsec = signer ? hexToBech32("nsec", signer.privateKey) : "";

  return (
    <div className="space-y-4">
      {step === "setup" && (
        <>
          <button
            className="text-xs text-neutral-500 hover:text-neutral-300 transition-colors"
            onClick={onBack}
          >
            ← Back
          </button>

          {/* Avatar + name row */}
          <div className="flex items-center gap-4">
            <button
              type="button"
              className="w-16 h-16 rounded-sm border border-neutral-700 bg-neutral-800 hover:border-neutral-500 transition-colors flex items-center justify-center overflow-hidden shrink-0"
              onClick={() => fileInputRef.current?.click()}
              title="Pick avatar"
            >
              {avatarPreview ? (
                <img
                  src={avatarPreview}
                  className="w-full h-full object-cover"
                  alt="avatar preview"
                />
              ) : (
                <svg
                  xmlns="http://www.w3.org/2000/svg"
                  width="24"
                  height="24"
                  viewBox="0 0 24 24"
                  fill="none"
                  stroke="currentColor"
                  strokeWidth="1.5"
                  strokeLinecap="round"
                  strokeLinejoin="round"
                  className="text-neutral-500"
                >
                  <circle cx="12" cy="8" r="4" />
                  <path d="M4 20c0-4 3.6-7 8-7s8 3 8 7" />
                </svg>
              )}
            </button>
            <div className="flex-1 space-y-1">
              <input
                type="text"
                className="w-full bg-neutral-800 border border-neutral-700 rounded-sm px-3 py-2 text-sm text-white placeholder-neutral-600 focus:outline-none focus:border-neutral-500"
                placeholder="Display name"
                value={name}
                onChange={(e) => setName(e.target.value)}
                autoFocus
                maxLength={64}
              />
              <p className="text-xs text-neutral-500">
                Click the avatar to upload a photo
              </p>
            </div>
          </div>

          <input
            ref={fileInputRef}
            type="file"
            accept="image/*"
            className="hidden"
            onChange={pickAvatar}
          />

          <textarea
            className="w-full bg-neutral-800 border border-neutral-700 rounded-sm px-3 py-2 text-sm text-white placeholder-neutral-600 focus:outline-none focus:border-neutral-500 resize-none"
            placeholder="About (optional)"
            rows={2}
            value={about}
            onChange={(e) => setAbout(e.target.value)}
            maxLength={256}
          />

          <Button className="w-full" onClick={generateKey}>
            Next
          </Button>

          <p className="text-xs text-neutral-600 text-center">
            Name and avatar can be changed any time after signup.
          </p>
        </>
      )}

      {step === "save-key" && (
        <>
          <button
            className="text-xs text-neutral-500 hover:text-neutral-300 transition-colors"
            onClick={() => setStep("setup")}
          >
            ← Back
          </button>

          <div className="space-y-2">
            <p className="text-sm font-medium text-white">
              Save your private key
            </p>
            <p className="text-xs text-neutral-400">
              This is the only time your key will be shown. Store it somewhere
              safe — anyone who has it controls your account.
            </p>
          </div>

          <div className="bg-neutral-800 border border-neutral-700 rounded-sm p-3 space-y-2">
            <p className="text-xs text-neutral-500 font-mono break-all select-all">
              {nsec}
            </p>
            <button
              className="text-xs text-neutral-400 hover:text-white transition-colors"
              onClick={copyNsec}
            >
              {copied ? "Copied!" : "Copy to clipboard"}
            </button>
          </div>

          {error && <p className="text-red-400 text-xs">{error}</p>}

          <Button className="w-full" onClick={publish}>
            I've saved my key — continue
          </Button>
        </>
      )}

      {step === "publishing" && (
        <div className="flex flex-col items-center gap-3 py-6">
          <div className="w-5 h-5 border-2 border-white border-t-transparent rounded-full animate-spin" />
          <p className="text-sm text-neutral-300">{statusMsg}</p>
        </div>
      )}
    </div>
  );
}
