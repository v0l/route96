import { base64 } from "@scure/base";
import { throwIfOffline } from "@snort/shared";
import { EventKind, EventPublisher, NostrEvent } from "@snort/system";
import { UploadProgressCallback, uploadWithProgress } from "./progress";

export class Nip96 {
  #info?: Nip96Info;

  constructor(
    readonly url: string,
    readonly publisher: EventPublisher,
  ) {
    this.url = new URL(this.url).toString();
  }

  async loadInfo() {
    const u = new URL(this.url);

    const rsp = await fetch(
      `${u.protocol}//${u.host}/.well-known/nostr/nip96.json`,
    );
    this.#info = (await rsp.json()) as Nip96Info;
    return this.#info;
  }

  async listFiles(page = 0, count = 10) {
    const rsp = await this.#req(`?page=${page}&count=${count}`, "GET");
    const data = await this.#handleResponse<Nip96FileList>(rsp);
    return data;
  }

  async upload(file: File, onProgress?: UploadProgressCallback) {
    const fd = new FormData();
    fd.append("size", file.size.toString());
    fd.append("caption", file.name);
    fd.append("content_type", file.type);
    fd.append("file", file);

    const rsp = await this.#req("", "POST", fd, onProgress);
    const data = await this.#handleResponse<Nip96Result>(rsp);
    if (data.status !== "success") {
      throw new Error(data.message);
    }
    return data;
  }

  async #handleResponse<T extends Nip96Status>(rsp: Response) {
    if (rsp.ok) {
      return (await rsp.json()) as T;
    } else {
      const text = await rsp.text();
      try {
        const obj = JSON.parse(text) as Nip96Result;
        throw new Error(obj.message);
      } catch {
        throw new Error(`Upload failed: ${text}`);
      }
    }
  }

  async #req(path: string, method: "GET" | "POST" | "DELETE", body?: BodyInit, onProgress?: UploadProgressCallback) {
    throwIfOffline();
    const auth = async (url: string, method: string) => {
      const auth = await this.publisher.generic((eb) => {
        return eb
          .kind(EventKind.HttpAuthentication)
          .tag(["u", url])
          .tag(["method", method]);
      });
      return `Nostr ${base64.encode(
        new TextEncoder().encode(JSON.stringify(auth)),
      )}`;
    };

    const info = this.#info ?? (await this.loadInfo());
    let u = info.api_url;
    if (u.startsWith("/")) {
      u = `${this.url}${u.slice(1)}`;
    }
    u += path;

    const requestHeaders = {
      accept: "application/json",
      authorization: await auth(u, method),
    };

    // Use progress-enabled upload for POST requests with FormData
    if (method === "POST" && body && onProgress) {
      return await uploadWithProgress(u, method, body, requestHeaders, onProgress);
    }

    // Fall back to regular fetch for other requests
    return await fetch(u, {
      method,
      body,
      headers: requestHeaders,
    });
  }
}

export interface Nip96Info {
  api_url: string;
  download_url?: string;
}

export interface Nip96Status {
  status: string;
  message?: string;
}

export type Nip96Result = Nip96Status & {
  processing_url?: string;
  nip94_event: NostrEvent;
};

export type Nip96FileList = Nip96Status & {
  count: number;
  total: number;
  page: number;
  files: Array<NostrEvent>;
};
