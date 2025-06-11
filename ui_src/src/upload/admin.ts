import { base64 } from "@scure/base";
import { throwIfOffline } from "@snort/shared";
import { EventKind, EventPublisher, NostrEvent } from "@snort/system";

export interface AdminSelf {
  is_admin: boolean;
  file_count: number;
  total_size: number;
  paid_until?: number;
  quota?: number;
  free_quota?: number;
  total_available_quota?: number;
}

export interface Report {
  id: number;
  file_id: string;
  reporter_id: number;
  event_json: string;
  created: string;
  reviewed: boolean;
}

export interface PaymentInfo {
  unit: string;
  interval: string;
  cost: {
    currency: string;
    amount: number;
  };
}

export interface PaymentRequest {
  units: number;
  quantity: number;
}

export interface PaymentResponse {
  pr: string;
}

export class Route96 {
  constructor(
    readonly url: string,
    readonly publisher: EventPublisher,
  ) {
    this.url = new URL(this.url).toString();
  }

  async getSelf() {
    const rsp = await this.#req("admin/self", "GET");
    const data = await this.#handleResponse<AdminResponse<AdminSelf>>(rsp);
    return data;
  }

  async listFiles(page = 0, count = 10, mime: string | undefined) {
    const rsp = await this.#req(
      `admin/files?page=${page}&count=${count}${mime ? `&mime_type=${mime}` : ""}`,
      "GET",
    );
    const data = await this.#handleResponse<AdminResponseFileList>(rsp);
    return {
      ...data,
      ...data.data,
      files: data.data.files,
    };
  }

  async listReports(page = 0, count = 10) {
    const rsp = await this.#req(
      `admin/reports?page=${page}&count=${count}`,
      "GET",
    );
    const data = await this.#handleResponse<AdminResponseReportList>(rsp);
    return {
      ...data,
      ...data.data,
      files: data.data.files,
    };
  }

  async acknowledgeReport(reportId: number) {
    const rsp = await this.#req(`admin/reports/${reportId}`, "DELETE");
    const data = await this.#handleResponse<AdminResponse<void>>(rsp);
    return data;
  }

  async getPaymentInfo() {
    const rsp = await this.#req("payment", "GET");
    if (rsp.ok) {
      return (await rsp.json()) as PaymentInfo;
    } else {
      const text = await rsp.text();
      try {
        const obj = JSON.parse(text) as AdminResponseBase;
        throw new Error(obj.message);
      } catch {
        throw new Error(`Payment info failed: ${text}`);
      }
    }
  }

  async requestPayment(request: PaymentRequest) {
    const rsp = await this.#req("payment", "POST", JSON.stringify(request));
    if (rsp.ok) {
      return (await rsp.json()) as PaymentResponse;
    } else {
      const text = await rsp.text();
      try {
        const obj = JSON.parse(text) as AdminResponseBase;
        throw new Error(obj.message);
      } catch {
        throw new Error(`Payment request failed: ${text}`);
      }
    }
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
    const headers: Record<string, string> = {
      accept: "application/json",
      authorization: await auth(u, method),
    };
    
    if (body && method !== "GET") {
      headers["content-type"] = "application/json";
    }

    return await fetch(u, {
      method,
      body,
      headers,
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

export type AdminResponseReportList = AdminResponse<{
  total: number;
  page: number;
  count: number;
  files: Array<Report>;
}>;
