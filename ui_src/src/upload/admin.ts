import { base64 } from "@scure/base";
import { throwIfOffline } from "@snort/shared";
import { EventKind, EventPublisher, NostrEvent } from "@snort/system";

export class Route96 {
  constructor(
    readonly url: string,
    readonly publisher: EventPublisher,
  ) {
    this.url = new URL(this.url).toString();
  }

  async getSelf() {
    const rsp = await this.#req("/admin/self", "GET");
    const data =
      await this.#handleResponse<AdminResponse<{ is_admin: boolean }>>(rsp);
    return data;
  }

  async listFiles(page = 0, count = 10) {
    const rsp = await this.#req(
      `/admin/files?page=${page}&count=${count}`,
      "GET",
    );
    const data = await this.#handleResponse<AdminResponseFileList>(rsp);
    return {
      ...data,
      ...data.data,
      files: data.data.files,
    };
  }

  async #handleResponse<T extends AdminResponseBase>(rsp: Response) {
    if (rsp.ok) {
      return (await rsp.json()) as T;
    } else {
      const text = await rsp.text();
      try {
        const obj = JSON.parse(text) as AdminResponseBase;
        throw new Error(obj.message);
      } catch {
        throw new Error(`Upload failed: ${text}`);
      }
    }
  }

  async #req(path: string, method: "GET" | "POST" | "DELETE", body?: BodyInit) {
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

    const u = `${this.url}${path}`;
    return await fetch(u, {
      method,
      body,
      headers: {
        accept: "application/json",
        authorization: await auth(u, method),
      },
    });
  }
}

export interface AdminResponseBase {
  status: string;
  message?: string;
}

export type AdminResponse<T> = AdminResponseBase & {
  data: T;
};

export type AdminResponseFileList = AdminResponse<{
  total: number;
  page: number;
  count: number;
  files: Array<NostrEvent>;
}>;
