import { base64 } from "@scure/base";
import { throwIfOffline } from "@snort/shared";
import { EventKind, EventPublisher } from "@snort/system";

export interface AdminSelf {
  is_admin: boolean;
  file_count: number;
  total_size: number;
  paid_until?: number;
  quota?: number;
  free_quota?: number;
  total_available_quota?: number;
}

export interface FileStats {
  last_accessed?: string;
  egress_bytes: number;
}

export interface AdminNip94File {
  created_at: number;
  content?: string;
  tags: Array<Array<string>>;
  uploader: Array<string>;
  stats?: FileStats;
}

export interface AdminUserInfo {
  pubkey: string;
  is_admin: boolean;
  file_count: number;
  total_size: number;
  created: string;
  paid_until?: number;
  quota?: number;
  free_quota?: number;
  total_available_quota?: number;
  payments?: any[];
  files: {
    total: number;
    page: number;
    count: number;
    files: Array<AdminNip94File>;
  };
}

export interface Report {
  id: number;
  file_id: string;
  reporter_id: number;
  event_json: string;
  created: string;
  reviewed: boolean;
}

export interface WhitelistEntry {
  pubkey: string;
  created: string;
}

export interface PaymentInfo {
  unit: string;
  interval: {
    [key: string]: number;
  };
  cost: {
    currency: string;
    amount: number;
  };
}

export interface SimilarFile {
  created_at: number;
  content?: string;
  tags: Array<Array<string>>;
  distance: number;
}

export interface PaymentRequest {
  units: number;
  quantity: number;
}

export interface PaymentResponse {
  pr: string;
}

export type FileStatSort = "created" | "egress_bytes" | "last_accessed";
export type SortOrder = "asc" | "desc";

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

  async listFiles(
    page = 0,
    count = 10,
    mime: string | undefined,
    label: string | undefined = undefined,
    sort: FileStatSort = "created",
    order: SortOrder = "desc",
  ) {
    const rsp = await this.#req(
      `admin/files?page=${page}&count=${count}${mime ? `&mime_type=${mime}` : ""}${label ? `&label=${encodeURIComponent(label)}` : ""}&sort=${sort}&order=${order}`,
      "GET",
    );
    const data = await this.#handleResponse<AdminResponseFileList>(rsp);
    if (!data.data) throw new Error(data.message || "List files failed");
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
    if (!data.data) throw new Error(data.message || "List reports failed");
    return {
      ...data,
      ...data.data,
      files: data.data.files,
    };
  }

  async acknowledgeReports(reportIds: number[]) {
    const rsp = await this.#req(
      "admin/reports",
      "DELETE",
      JSON.stringify({ ids: reportIds }),
    );
    const data = await this.#handleResponse<AdminResponse<void>>(rsp);
    return data;
  }

  async getUserInfo(pubkey: string, page = 0, count = 50) {
    const rsp = await this.#req(
      `admin/user/${pubkey}?page=${page}&count=${count}`,
      "GET",
    );
    const data = await this.#handleResponse<AdminResponse<AdminUserInfo>>(rsp);
    return data;
  }

  async purgeUser(pubkey: string) {
    const rsp = await this.#req(`admin/user/${pubkey}/purge`, "DELETE");
    const data = await this.#handleResponse<AdminResponse<void>>(rsp);
    return data;
  }

  async listPendingReview(page = 0, count = 50) {
    const rsp = await this.#req(
      `admin/files/review?page=${page}&count=${count}`,
      "GET",
    );
    const data = await this.#handleResponse<AdminResponseFileList>(rsp);
    if (!data.data)
      throw new Error(data.message || "List pending review failed");
    return {
      ...data,
      ...data.data,
      files: data.data.files,
    };
  }

  async reviewFiles(fileIds: string[]) {
    const rsp = await this.#req(
      "admin/files/review",
      "PATCH",
      JSON.stringify({ ids: fileIds }),
    );
    const data = await this.#handleResponse<AdminResponse<void>>(rsp);
    return data;
  }

  async deleteReviewFiles(fileIds: string[]) {
    const rsp = await this.#req(
      "admin/files/review",
      "DELETE",
      JSON.stringify({ ids: fileIds }),
    );
    const data = await this.#handleResponse<AdminResponse<void>>(rsp);
    return data;
  }

  async findSimilar(fileId: string, distance?: number) {
    const params = distance !== undefined ? `?distance=${distance}` : "";
    const rsp = await this.#req(
      `admin/files/${fileId}/similar${params}`,
      "GET",
    );
    const data =
      await this.#handleResponse<AdminResponse<Array<SimilarFile>>>(rsp);
    if (!data.data) throw new Error(data.message || "Find similar failed");
    return data.data;
  }

  async listWhitelist() {
    const rsp = await this.#req("admin/whitelist", "GET");
    const data =
      await this.#handleResponse<AdminResponse<WhitelistEntry[]>>(rsp);
    if (!data.data) throw new Error(data.message || "List whitelist failed");
    return data.data;
  }

  async addToWhitelist(pubkey: string) {
    const rsp = await this.#req(
      "admin/whitelist",
      "POST",
      JSON.stringify({ pubkey }),
    );
    const data = await this.#handleResponse<AdminResponse<void>>(rsp);
    return data;
  }

  async removeFromWhitelist(pubkey: string) {
    const rsp = await this.#req(
      "admin/whitelist",
      "DELETE",
      JSON.stringify({ pubkey }),
    );
    const data = await this.#handleResponse<AdminResponse<void>>(rsp);
    return data;
  }

  async getPaymentInfo() {
    const rsp = await this.#req("payment", "GET");
    if (rsp.ok) {
      return (await rsp.json()) as PaymentInfo;
    }
    throw new Error(
      rsp.headers.get("X-Reason") ||
        (await rsp.text()) ||
        `${rsp.status} ${rsp.statusText}`,
    );
  }

  async requestPayment(request: PaymentRequest) {
    const rsp = await this.#req("payment", "POST", JSON.stringify(request));
    if (rsp.ok) {
      return (await rsp.json()) as PaymentResponse;
    }
    throw new Error(
      rsp.headers.get("X-Reason") ||
        (await rsp.text()) ||
        `${rsp.status} ${rsp.statusText}`,
    );
  }

  async #handleResponse<T extends AdminResponseBase>(rsp: Response) {
    if (rsp.ok) {
      return (await rsp.json()) as T;
    }
    throw new Error(
      rsp.headers.get("X-Reason") ||
        (await rsp.text()) ||
        `${rsp.status} ${rsp.statusText}`,
    );
  }

  async #req(
    path: string,
    method: "GET" | "POST" | "DELETE" | "PATCH",
    body?: BodyInit,
  ) {
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
  data?: T;
};

export type AdminResponseFileList = AdminResponse<{
  total: number;
  page: number;
  count: number;
  files: Array<AdminNip94File>;
}>;

export type AdminResponseReportList = AdminResponse<{
  total: number;
  page: number;
  count: number;
  files: Array<Report>;
}>;
