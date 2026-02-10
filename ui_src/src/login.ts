import { ExternalStore } from "@snort/shared";
import {
  EventPublisher,
  Nip46Signer,
  Nip7Signer,
  PrivateKeySigner,
} from "@snort/system";

export interface LoginSession {
  type: "nip7" | "nsec" | "nip46";
  publicKey: string;
  privateKey?: string;
  bunker?: string;
  currency: string;
}

// Helper to wait for window.nostr to be available
async function waitForNostr(maxWaitMs = 3000): Promise<boolean> {
  const startTime = Date.now();
  while (Date.now() - startTime < maxWaitMs) {
    if (window.nostr) {
      return true;
    }
    await new Promise(resolve => setTimeout(resolve, 100));
  }
  return false;
}

class LoginStore extends ExternalStore<LoginSession | undefined> {
  #session?: LoginSession;
  #signer?: EventPublisher;
  #signerInitPromise?: Promise<EventPublisher | undefined>;

  constructor() {
    super();
    const s = window.localStorage.getItem("session");
    if (s) {
      this.#session = JSON.parse(s);
      // patch session
      if (this.#session) {
        this.#session.type ??= "nip7";
      }
    }
  }

  takeSnapshot() {
    return this.#session ? { ...this.#session } : undefined;
  }

  logout() {
    this.#session = undefined;
    this.#signer = undefined;
    this.#signerInitPromise = undefined;
    this.#save();
  }

  login(pubkey: string, type: LoginSession["type"] = "nip7") {
    this.#session = {
      type: type ?? "nip7",
      publicKey: pubkey,
      currency: "EUR",
    };
    this.#signer = undefined;
    this.#signerInitPromise = undefined;
    this.#save();
  }

  loginPrivateKey(key: string) {
    const s = new PrivateKeySigner(key);
    this.#session = {
      type: "nsec",
      publicKey: s.getPubKey(),
      privateKey: key,
      currency: "EUR",
    };
    this.#save();
  }

  loginBunker(url: string, localKey: string, remotePubkey: string) {
    this.#session = {
      type: "nip46",
      publicKey: remotePubkey,
      privateKey: localKey,
      bunker: url,
      currency: "EUR",
    };
    this.#save();
  }

  getSigner() {
    if (!this.#signer && this.#session) {
      switch (this.#session.type) {
        case "nsec":
          this.#signer = new EventPublisher(
            new PrivateKeySigner(this.#session.privateKey!),
            this.#session.publicKey,
          );
          break;
        case "nip46":
          this.#signer = new EventPublisher(
            new Nip46Signer(
              this.#session.bunker!,
              new PrivateKeySigner(this.#session.privateKey!),
            ),
            this.#session.publicKey,
          );
          break;
        case "nip7":
          // For nip7, check if window.nostr is available
          if (!window.nostr) {
            throw new Error("NIP-07 extension not found. Please install a Nostr browser extension.");
          }
          this.#signer = new EventPublisher(
            new Nip7Signer(),
            this.#session.publicKey,
          );
          break;
      }
    }

    if (this.#signer) {
      return this.#signer;
    }
    throw new Error("Signer not setup!");
  }

  // Async version that waits for nip7 extension to load
  async getSignerAsync(): Promise<EventPublisher> {
    if (this.#signer) {
      return this.#signer;
    }

    if (this.#signerInitPromise) {
      const result = await this.#signerInitPromise;
      if (result) return result;
      throw new Error("Signer not setup!");
    }

    if (!this.#session) {
      throw new Error("Signer not setup!");
    }

    this.#signerInitPromise = this.#initSignerAsync();
    const result = await this.#signerInitPromise;
    if (result) return result;
    throw new Error("Signer not setup!");
  }

  async #initSignerAsync(): Promise<EventPublisher | undefined> {
    if (!this.#session) return undefined;

    switch (this.#session.type) {
      case "nsec":
        this.#signer = new EventPublisher(
          new PrivateKeySigner(this.#session.privateKey!),
          this.#session.publicKey,
        );
        break;
      case "nip46":
        this.#signer = new EventPublisher(
          new Nip46Signer(
            this.#session.bunker!,
            new PrivateKeySigner(this.#session.privateKey!),
          ),
          this.#session.publicKey,
        );
        break;
      case "nip7":
        // Wait for window.nostr to become available
        const nostrAvailable = await waitForNostr();
        if (!nostrAvailable) {
          throw new Error("NIP-07 extension not found. Please install a Nostr browser extension.");
        }
        this.#signer = new EventPublisher(
          new Nip7Signer(),
          this.#session.publicKey,
        );
        break;
    }

    return this.#signer;
  }

  updateSession(fx: (s: LoginSession) => void) {
    if (this.#session) {
      fx(this.#session);
      this.#save();
    }
  }

  #save() {
    if (this.#session) {
      window.localStorage.setItem("session", JSON.stringify(this.#session));
    } else {
      window.localStorage.removeItem("session");
    }
    this.notifyChange();
  }
}

export const Login = new LoginStore();