import { base64, bytesToString } from "@scure/base";
import { throwIfOffline, unixNow } from "@snort/shared";
import { EventKind, EventPublisher } from "@snort/system";

export interface BlobDescriptor {
  url?: string;
  sha256: string;
  size: number;
  type?: string;
  uploaded?: number;
}

export class Blossom {
  constructor(
    readonly url: string,
    readonly publisher: EventPublisher,
  ) {
    this.url = new URL(this.url).toString();
  }

  async upload(file: File) {
    const hash = await window.crypto.subtle.digest(
      "SHA-256",
      await file.arrayBuffer(),
    );
    const tags = [["x", bytesToString("hex", new Uint8Array(hash))]];

    const rsp = await this.#req("/upload", "PUT", file, tags);
    if (rsp.ok) {
      return (await rsp.json()) as BlobDescriptor;
    } else {
      const text = await rsp.text();
      throw new Error(text);
    }
  }

  async #req(
    path: string,
    method: "GET" | "POST" | "DELETE" | "PUT",
    body?: BodyInit,
    tags?: Array<Array<string>>,
  ) {
    throwIfOffline();

    const url = `${this.url}upload`;
    const now = unixNow();
    const auth = async (url: string, method: string) => {
      const auth = await this.publisher.generic((eb) => {
        eb.kind(24_242 as EventKind)
          .tag(["u", url])
          .tag(["method", method])
          .tag(["t", path.slice(1)])
          .tag(["expiration", (now + 10).toString()]);
        tags?.forEach((t) => eb.tag(t));
        return eb;
      });
      return `Nostr ${base64.encode(
        new TextEncoder().encode(JSON.stringify(auth)),
      )}`;
    };

    return await fetch(url, {
      method,
      body,
      headers: {
        accept: "application/json",
        authorization: await auth(url, method),
      },
    });
  }
}
