import { base64 } from "@scure/base";
import { throwIfOffline, unixNow } from "@snort/shared";
import { EventKind, EventPublisher } from "@snort/system";

export interface AdminSelf {
  is_admin: boolean;
  setup_mode: boolean;
  file_count: number;
  total_size: number;
  paid_until?: number;
  quota?: number;
  free_quota?: number;
  total_available_quota?: number;
}

export interface SetupRequest {
  public_url: string;
  max_upload_bytes?: number;
}

export interface FileStats {
  last_accessed?: string;
  egress_bytes: number;
}

export interface Route96File {
  created_at: number;
  content?: string;
  tags: Array<Array<string>>;
  uploader?: Array<string>;
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
    files: Array<Route96File>;
  };
}

export interface LabelModel {
  name: string;
  type: string;
  config: string;
}

export interface AddLabelModelRequest {
  name: string;
  model_type: string;
  hf_repo?: string;
  api_url?: string;
  llm_model?: string;
  api_key?: string;
  prompt?: string;
  label_exclude?: string;
  min_confidence?: string;
}

export interface LabelFlagTermsResponse {
  terms: string[];
}

export interface LabelModelsResponse {
  models: LabelModel[];
}

export interface DailyStat {
  date: string;
  uploads: number;
  bytes: number;
}

export interface AdminStatsResponse {
  days: number;
  stats: DailyStat[];
}

export interface BackgroundTaskProgress {
  task: string;
  pending: number;
  total: number;
  percent: number;
}

export interface BackgroundProgressResponse {
  tasks: BackgroundTaskProgress[];
  total_pending: number;
  total_percent: number;
}


export interface Report {
  id: number;
  file_id: string;
  reporter_id: number;
  event_json: string;
  created: string;
  reviewed: boolean;
}

export interface GroupedReport {
  file_id: string;
  report_count: number;
  latest_report_id: number;
  reporter_pubkey: string;
  reason: string;
  created: string;
}

export interface WhitelistEntry {
  pubkey: string;
  created: string;
}

export interface ConfigEntry {
  key: string;
  value: string;
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

  /** Submit initial setup configuration (NIP-98 authenticated). */
  async postSetup(body: SetupRequest) {
    const rsp = await this.#req("setup", "POST", JSON.stringify(body));
    return this.#handleResponse<AdminResponse<void>>(rsp);
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

  async deleteReports(reportIds: number[]) {
    const rsp = await this.#req(
      "admin/reports/bulk",
      "DELETE",
      JSON.stringify({ ids: reportIds }),
    );
    const data = await this.#handleResponse<AdminResponse<void>>(rsp);
    return data;
  }

  async listReportsGrouped(page = 0, count = 10) {
    const rsp = await this.#req(
      `admin/reports/grouped?page=${page}&count=${count}`,
      "GET",
    );
    const data = await this.#handleResponse<AdminResponseGroupedReportList>(rsp);
    if (!data.data) throw new Error(data.message || "List reports failed");
    return {
      ...data,
      ...data.data,
      files: data.data.files,
    };
  }

  async deleteFilesBulk(fileIds: string[]) {
    const rsp = await this.#req(
      "admin/files/bulk",
      "DELETE",
      JSON.stringify({ ids: fileIds }),
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

  async listConfig() {
    const rsp = await this.#req("admin/config", "GET");
    const data = await this.#handleResponse<AdminResponse<ConfigEntry[]>>(rsp);
    if (!data.data) throw new Error(data.message || "List config failed");
    return data.data;
  }

  async setConfig(key: string, value: string) {
    const rsp = await this.#req(
      `admin/config/${encodeURIComponent(key)}`,
      "PUT",
      JSON.stringify({ value }),
    );
    return this.#handleResponse<AdminResponse<void>>(rsp);
  }

  async deleteConfig(key: string) {
    const rsp = await this.#req(
      `admin/config/${encodeURIComponent(key)}`,
      "DELETE",
    );
    return this.#handleResponse<AdminResponse<void>>(rsp);
  }

  // Label models API
  async listLabelModels() {
    const rsp = await this.#req("admin/label-models", "GET");
    const data = await this.#handleResponse<AdminResponse<LabelModel[]>>(rsp);
    if (!data.data) throw new Error(data.message || "List label models failed");
    return data.data;
  }

  async addLabelModel(model: AddLabelModelRequest) {
    const rsp = await this.#req(
      "admin/label-models",
      "POST",
      JSON.stringify(model),
    );
    return this.#handleResponse<AdminResponse<void>>(rsp);
  }

  async removeLabelModel(name: string) {
    const rsp = await this.#req(
      `admin/label-models/${encodeURIComponent(name)}`,
      "DELETE",
    );
    return this.#handleResponse<AdminResponse<void>>(rsp);
  }

  // Label flag terms API
  async getLabelFlagTerms() {
    const rsp = await this.#req("admin/label-flag-terms", "GET");
    const data = await this.#handleResponse<AdminResponse<string[] | null>>(rsp);
    return data.data;
  }

  async setLabelFlagTerms(terms: string[]) {
    const rsp = await this.#req(
      "admin/label-flag-terms",
      "PUT",
      JSON.stringify({ terms }),
    );
    return this.#handleResponse<AdminResponse<void>>(rsp);
  }

  async deleteLabelFlagTerms() {
    const rsp = await this.#req("admin/label-flag-terms", "DELETE");
    return this.#handleResponse<AdminResponse<void>>(rsp);
  }

  async getStats(days: number = 30) {
    const rsp = await this.#req(`admin/stats?days=${days}`, "GET");
    const data = await this.#handleResponse<AdminResponse<AdminStatsResponse>>(rsp);
    if (!data.data) throw new Error(data.message || "Get stats failed");
    return data.data;
  }

  async getBackgroundProgress() {
    const rsp = await this.#req("admin/background-progress", "GET");
    const data = await this.#handleResponse<AdminResponse<BackgroundProgressResponse>>(rsp);
    if (!data.data) throw new Error(data.message || "Get background progress failed");
    return data.data;
  }

  async listUserFiles(
    page = 0,
    count = 50,
    mime?: string,
    label?: string,
    sort: FileStatSort = "created",
    order: SortOrder = "desc",
  ) {
    const params = new URLSearchParams({
      page: page.toString(),
      count: count.toString(),
      sort,
      order,
    });
    if (mime) params.set("mime_type", mime);
    if (label) params.set("label", label);
    const rsp = await this.#blossomReq(
      `user/files?${params}`,
      "GET",
      "list",
    );
    const data = await this.#handleResponse<AdminResponseFileList>(rsp);
    if (!data.data) throw new Error(data.message || "List files failed");
    return {
      ...data,
      ...data.data,
      files: data.data.files,
    };
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

  /** Make a request authenticated with a Blossom kind-24242 event. */
  async #blossomReq(
    path: string,
    method: "GET" | "POST" | "DELETE" | "PATCH",
    term: string,
    body?: BodyInit,
  ) {
    throwIfOffline();
    const u = `${this.url}${path}`;
    const now = unixNow();
    const auth = await this.publisher.generic((eb) => {
      return eb
        .kind(24_242 as EventKind)
        .tag(["t", term])
        .tag(["expiration", (now + 60).toString()]);
    });
    const headers: Record<string, string> = {
      accept: "application/json",
      authorization: `Nostr ${base64.encode(new TextEncoder().encode(JSON.stringify(auth)))}`,
    };
    if (body && method !== "GET") {
      headers["content-type"] = "application/json";
    }
    return await fetch(u, { method, body, headers });
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
    method: "GET" | "POST" | "PUT" | "DELETE" | "PATCH",
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
  files: Array<Route96File>;
}>;

export type AdminResponseReportList = AdminResponse<{
  total: number;
  page: number;
  count: number;
  files: Array<Report>;
}>;

export type AdminResponseGroupedReportList = AdminResponse<{
  total: number;
  page: number;
  count: number;
  files: Array<GroupedReport>;
}>;
