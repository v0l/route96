import { base64, hex } from "@scure/base";
import { throwIfOffline, unixNow } from "@snort/shared";
import { EventKind, EventPublisher } from "@snort/system";
import { UploadProgressCallback, uploadWithProgress } from "./progress";
import { sha256 } from "@noble/hashes/sha2.js";

export interface BlobDescriptor {
  url?: string;
  sha256: string;
  size: number;
  type?: string;
  uploaded?: number;
}

/** Thrown when the server returns 409 with X-Identical-Media (BUD-12). */
export class IdenticalMediaError extends Error {
  /** SHA-256 of the existing equivalent blob on the server. */
  readonly existingSha256: string;

  constructor(existingSha256: string, reason?: string) {
    super(reason || "An identical image already exists on this server.");
    this.name = "IdenticalMediaError";
    this.existingSha256 = existingSha256;
  }
}

export class Blossom {
  constructor(
    readonly url: string,
    readonly publisher: EventPublisher,
  ) {
    this.url = new URL(this.url).toString();
  }

  async #handleError(rsp: Response) {
    throw new Error(
      rsp.headers.get("X-Reason") ||
      (await rsp.text()) ||
      `${rsp.status} ${rsp.statusText}`,
    );
  }

  async #handleUploadResponse(rsp: Response): Promise<BlobDescriptor> {
    if (rsp.ok) {
      return (await rsp.json()) as BlobDescriptor;
    }
    if (rsp.status === 409) {
      const existingSha256 = rsp.headers.get("X-Identical-Media");
      if (existingSha256) {
        const reason = rsp.headers.get("X-Reason") ?? undefined;
        throw new IdenticalMediaError(existingSha256, reason);
      }
    }
    await this.#handleError(rsp);
    throw new Error("Should not reach here");
  }

  async sha256(file: File): Promise<Uint8Array> {
    if (window.crypto?.subtle?.digest !== undefined) {
      return new Uint8Array(await window.crypto.subtle.digest(
        "SHA-256",
        await file.arrayBuffer(),
      ));
    } else {
      return sha256(new Uint8Array(await file.arrayBuffer()));
    }
  }

  async upload(
    file: File,
    onProgress?: UploadProgressCallback,
    acknowledgedSha256?: string,
  ): Promise<BlobDescriptor> {
    const hash = await this.sha256(file);
    const tags = [["x", hex.encode(hash)]];

    const rsp = await this.#req(
      "upload",
      "PUT",
      "upload",
      file,
      tags,
      acknowledgedSha256 ? { "x-identical-media": acknowledgedSha256 } : undefined,
      onProgress,
    );
    return this.#handleUploadResponse(rsp);
  }

  async media(
    file: File,
    onProgress?: UploadProgressCallback,
    acknowledgedSha256?: string,
  ): Promise<BlobDescriptor> {
    const hash = await this.sha256(file);
    const tags = [["x", hex.encode(new Uint8Array(hash))]];

    const rsp = await this.#req(
      "media",
      "PUT",
      "media",
      file,
      tags,
      acknowledgedSha256 ? { "x-identical-media": acknowledgedSha256 } : undefined,
      onProgress,
    );
    return this.#handleUploadResponse(rsp);
  }

  async mirror(url: string): Promise<BlobDescriptor> {
    const rsp = await this.#req(
      "mirror",
      "PUT",
      "upload",
      JSON.stringify({ url }),
      undefined,
      {
        "content-type": "application/json",
      },
    );
    return this.#handleUploadResponse(rsp);
  }

  async list(pk: string): Promise<Array<BlobDescriptor>> {
    const rsp = await this.#req(`list/${pk}`, "GET", "list");
    if (rsp.ok) {
      return (await rsp.json()) as Array<BlobDescriptor>;
    } else {
      await this.#handleError(rsp);
      throw new Error("Should not reach here");
    }
  }

  async delete(id: string): Promise<void> {
    const tags = [["x", id]];

    const rsp = await this.#req(id, "DELETE", "delete", undefined, tags);
    if (!rsp.ok) {
      await this.#handleError(rsp);
      throw new Error("Should not reach here");
    }
  }

  async #req(
    path: string,
    method: "GET" | "POST" | "DELETE" | "PUT",
    term: string,
    body?: BodyInit,
    tags?: Array<Array<string>>,
    headers?: Record<string, string>,
    onProgress?: UploadProgressCallback,
  ) {
    throwIfOffline();

    const url = `${this.url}${path}`;
    const now = unixNow();
    const auth = async (url: string, method: string) => {
      const auth = await this.publisher.generic((eb) => {
        eb.kind(24_242 as EventKind)
          .tag(["u", url])
          .tag(["method", method.toLowerCase()])
          .tag(["t", term])
          .tag(["expiration", (now + 10).toString()]);
        tags?.forEach((t) => eb.tag(t));
        return eb;
      });
      return `Nostr ${base64.encode(
        new TextEncoder().encode(JSON.stringify(auth)),
      )}`;
    };

    const requestHeaders = {
      ...headers,
      accept: "application/json",
      authorization: await auth(url, method),
    };

    // Use progress-enabled upload for PUT requests with body
    if (method === "PUT" && body && onProgress) {
      return await uploadWithProgress(
        url,
        method,
        body,
        requestHeaders,
        onProgress,
      );
    }

    // Fall back to regular fetch for other requests
    return await fetch(url, {
      method,
      body,
      headers: requestHeaders,
    });
  }
}
