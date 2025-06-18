import { base64, bytesToString } from "@scure/base";
import { throwIfOffline, unixNow } from "@snort/shared";
import { EventKind, EventPublisher } from "@snort/system";
import { UploadProgressCallback, uploadWithProgress } from "./progress";

export interface BlobDescriptor {
  url?: string;
  sha256: string;
  size: number;
  type?: string;
  uploaded?: number;
}

export interface FileMirrorSuggestion {
  sha256: string;
  url: string;
  size: number;
  mime_type?: string;
  available_on: string[];
  missing_from: string[];
}

export interface MirrorSuggestionsResponse {
  suggestions: FileMirrorSuggestion[];
}

export class Blossom {
  constructor(
    readonly url: string,
    readonly publisher: EventPublisher,
  ) {
    this.url = new URL(this.url).toString();
  }

  async #handleError(rsp: Response) {
    const reason = rsp.headers.get("X-Reason") || rsp.headers.get("x-reason");
    if (reason) {
      throw new Error(reason);
    } else {
      const text = await rsp.text();
      throw new Error(text);
    }
  }

  async upload(file: File, onProgress?: UploadProgressCallback): Promise<BlobDescriptor> {
    const hash = await window.crypto.subtle.digest(
      "SHA-256",
      await file.arrayBuffer(),
    );
    const tags = [["x", bytesToString("hex", new Uint8Array(hash))]];

    const rsp = await this.#req("upload", "PUT", "upload", file, tags, undefined, onProgress);
    if (rsp.ok) {
      return (await rsp.json()) as BlobDescriptor;
    } else {
      await this.#handleError(rsp);
      throw new Error("Should not reach here");
    }
  }

  async media(file: File, onProgress?: UploadProgressCallback): Promise<BlobDescriptor> {
    const hash = await window.crypto.subtle.digest(
      "SHA-256",
      await file.arrayBuffer(),
    );
    const tags = [["x", bytesToString("hex", new Uint8Array(hash))]];

    const rsp = await this.#req("media", "PUT", "media", file, tags, undefined, onProgress);
    if (rsp.ok) {
      return (await rsp.json()) as BlobDescriptor;
    } else {
      await this.#handleError(rsp);
      throw new Error("Should not reach here");
    }
  }

  async mirror(url: string): Promise<BlobDescriptor> {
    const rsp = await this.#req(
      "mirror",
      "PUT",
      "mirror",
      JSON.stringify({ url }),
      undefined,
      {
        "content-type": "application/json",
      },
    );
    if (rsp.ok) {
      return (await rsp.json()) as BlobDescriptor;
    } else {
      await this.#handleError(rsp);
      throw new Error("Should not reach here");
    }
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

  async getMirrorSuggestions(servers: string[]): Promise<MirrorSuggestionsResponse> {
    const rsp = await this.#req(
      "mirror-suggestions",
      "POST",
      "mirror-suggestions",
      JSON.stringify({ servers }),
      undefined,
      {
        "content-type": "application/json",
      },
    );
    if (rsp.ok) {
      return (await rsp.json()) as MirrorSuggestionsResponse;
    } else {
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
      return await uploadWithProgress(url, method, body, requestHeaders, onProgress);
    }

    // Fall back to regular fetch for other requests
    return await fetch(url, {
      method,
      body,
      headers: requestHeaders,
    });
  }
}
