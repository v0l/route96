import { useEffect, useState, useCallback } from "react";
import { Navigate } from "react-router-dom";
import classNames from "classnames";
import FileList, { type FileInfo } from "./files";
import ReportList from "./reports";
import useLogin from "../hooks/login";
import usePublisher from "../hooks/publisher";
import {
  AdminSelf,
  Route96File,
  Route96,
  Report,
  SimilarFile,
  ConfigEntry,
  FileStatSort,
  SortOrder,
} from "../upload/admin";
import { Blossom } from "../upload/blossom";
import { FormatBytes } from "../const";
import FileListControls from "../components/file-list-controls";
import ConfigEditor from "../components/config-editor";
import LabelManagement from "../components/label-management";
import Stats from "../components/stats";
import BackgroundProgress from "../components/background-progress";

type Tab = "files" | "reports" | "review" | "config" | "labeling" | "stats" | "progress";

type AdminFileList = {
  count: number;
  total: number;
  page: number;
  files: Array<Route96File>;
};

export default function Admin() {
  const [self, setSelf] = useState<AdminSelf>();
  const [error, setError] = useState<string>();
  const [tab, setTab] = useState<Tab>("files");
  const [loading, setLoading] = useState(true);

  // Files tab
  const [adminListedFiles, setAdminListedFiles] = useState<AdminFileList>();
  const [adminListedPage, setAdminListedPage] = useState(0);
  const [mimeFilter, setMimeFilter] = useState<string>();
  const [labelFilter, setLabelFilter] = useState<string>();
  const [sortBy, setSortBy] = useState<FileStatSort>("created");
  const [sortOrder, setSortOrder] = useState<SortOrder>("desc");
  const [bulkProgress, setBulkProgress] = useState<string>();

  // Reports tab
  const [reports, setReports] = useState<Report[]>();
  const [reportPages, setReportPages] = useState<number>();
  const [reportPage, setReportPage] = useState(0);

  // Review tab
  const [pendingReview, setPendingReview] = useState<AdminFileList>();
  const [pendingReviewPage, setPendingReviewPage] = useState(0);

  // Config tab
  const [config, setConfig] = useState<ConfigEntry[]>();

  // Similar images modal
  const [similarFiles, setSimilarFiles] = useState<SimilarFile[]>();
  const [similarLoading, setSimilarLoading] = useState(false);
  const [similarSource, setSimilarSource] = useState<FileInfo>();

  const login = useLogin();
  const pub = usePublisher();

  const url =
    import.meta.env.VITE_API_URL || `${location.protocol}//${location.host}`;

  const listAllUploads = useCallback(
    async (n: number) => {
      if (!pub) return;
      try {
        setError(undefined);
        const route96 = new Route96(url, pub);
        const result = await route96.listFiles(
          n,
          50,
          mimeFilter,
          labelFilter,
          sortBy,
          sortOrder,
        );
        setAdminListedFiles(result);
      } catch (e) {
        setError(
          e instanceof Error
            ? e.message || "List files failed"
            : "List files failed",
        );
      }
    },
    [pub, url, mimeFilter, labelFilter, sortBy, sortOrder],
  );

  const listReports = useCallback(
    async (n: number) => {
      if (!pub) return;
      try {
        setError(undefined);
        const route96 = new Route96(url, pub);
        const result = await route96.listReports(n, 10);
        setReports(result.files);
        setReportPages(Math.ceil(result.total / result.count));
      } catch (e) {
        setError(
          e instanceof Error
            ? e.message || "List reports failed"
            : "List reports failed",
        );
      }
    },
    [pub, url],
  );

  const listPendingReview = useCallback(
    async (n: number) => {
      if (!pub) return;
      try {
        setError(undefined);
        const route96 = new Route96(url, pub);
        const result = await route96.listPendingReview(n, 50);
        setPendingReview(result);
      } catch (e) {
        setError(
          e instanceof Error
            ? e.message || "List pending review failed"
            : "List pending review failed",
        );
      }
    },
    [pub, url],
  );

  const listConfig = useCallback(async () => {
    if (!pub) return;
    try {
      setError(undefined);
      const route96 = new Route96(url, pub);
      setConfig(await route96.listConfig());
    } catch (e) {
      setError(e instanceof Error ? e.message || "List config failed" : "List config failed");
    }
  }, [pub, url]);

  async function saveConfig(key: string, value: string) {
    if (!pub) return;
    try {
      setError(undefined);
      const route96 = new Route96(url, pub);
      await route96.setConfig(key, value);
      await listConfig();
    } catch (e) {
      setError(e instanceof Error ? e.message || "Save config failed" : "Save config failed");
    }
  }

  async function deleteConfig(key: string) {
    if (!pub) return;
    try {
      setError(undefined);
      const route96 = new Route96(url, pub);
      await route96.deleteConfig(key);
      await listConfig();
    } catch (e) {
      setError(e instanceof Error ? e.message || "Delete config failed" : "Delete config failed");
    }
  }

  async function acknowledgeReport(reportId: number) {
    if (!pub) return;
    try {
      setError(undefined);
      const route96 = new Route96(url, pub);
      await route96.acknowledgeReports([reportId]);
      await listReports(reportPage);
    } catch (e) {
      setError(
        e instanceof Error
          ? e.message || "Acknowledge report failed"
          : "Acknowledge report failed",
      );
    }
  }

  async function reviewFile(id: string) {
    if (!pub) return;
    try {
      setError(undefined);
      const route96 = new Route96(url, pub);
      await route96.reviewFiles([id]);
      await listPendingReview(pendingReviewPage);
    } catch (e) {
      setError(
        e instanceof Error
          ? e.message || "Review file failed"
          : "Review file failed",
      );
    }
  }

  async function banFile(id: string) {
    if (!pub) return;
    try {
      setError(undefined);
      const route96 = new Route96(url, pub);
      await route96.deleteReviewFiles([id]);
      await listPendingReview(pendingReviewPage);
    } catch (e) {
      setError(
        e instanceof Error ? e.message || "Ban file failed" : "Ban file failed",
      );
    }
  }

  async function reviewAndDeleteFile(id: string) {
    if (!pub) return;
    try {
      setError(undefined);
      const route96 = new Route96(url, pub);
      await route96.reviewFiles([id]);
      const blossom = new Blossom(url, pub);
      await blossom.delete(id);
      await listPendingReview(pendingReviewPage);
    } catch (e) {
      setError(
        e instanceof Error
          ? e.message || "Delete failed"
          : "Delete file failed",
      );
    }
  }

  async function deleteFile(id: string) {
    if (!pub) return;
    try {
      setError(undefined);
      const blossom = new Blossom(url, pub);
      await blossom.delete(id);
    } catch (e) {
      setError(
        e instanceof Error
          ? e.message || "Delete failed"
          : "Delete file failed",
      );
    }
  }

  async function bulkApproveAll() {
    if (!pub || !pendingReview) return;
    const ids = pendingReview.files
      .map((f) => f.tags.find((t) => t[0] === "x")?.[1])
      .filter((id): id is string => !!id);
    if (ids.length === 0) return;
    const route96 = new Route96(url, pub);
    try {
      setBulkProgress(`Approving ${ids.length} files...`);
      await route96.reviewFiles(ids);
    } catch {
      // best-effort
    }
    setBulkProgress(undefined);
    await listPendingReview(pendingReviewPage);
  }

  async function bulkBanAll() {
    if (!pub || !pendingReview) return;
    const ids = pendingReview.files
      .map((f) => f.tags.find((t) => t[0] === "x")?.[1])
      .filter((id): id is string => !!id);
    if (ids.length === 0) return;
    const route96 = new Route96(url, pub);
    try {
      setBulkProgress(`Banning ${ids.length} files...`);
      await route96.deleteReviewFiles(ids);
    } catch {
      // best-effort
    }
    setBulkProgress(undefined);
    await listPendingReview(pendingReviewPage);
  }

  async function findSimilar(file: FileInfo) {
    if (!pub) return;
    try {
      setError(undefined);
      setSimilarLoading(true);
      setSimilarSource(file);
      const route96 = new Route96(url, pub);
      const result = await route96.findSimilar(file.id);
      setSimilarFiles(result);
    } catch (e) {
      setError(
        e instanceof Error
          ? e.message || "Find similar failed"
          : "Find similar failed",
      );
      setSimilarFiles(undefined);
      setSimilarSource(undefined);
    } finally {
      setSimilarLoading(false);
    }
  }

  function closeSimilarModal() {
    setSimilarFiles(undefined);
    setSimilarSource(undefined);
  }

  useEffect(() => {
    if (pub) {
      const r96 = new Route96(url, pub);
      r96
        .getSelf()
        .then((v) => {
          setSelf(v.data);
          setLoading(false);
        })
        .catch(() => setLoading(false));
    }
  }, [login, pub, url]);

  // Load data when tab becomes active
  useEffect(() => {
    if (pub && self?.is_admin && tab === "files") {
      listAllUploads(adminListedPage);
    }
  }, [tab, adminListedPage, pub, self?.is_admin, listAllUploads]);

  useEffect(() => {
    if (pub && self?.is_admin && tab === "reports") {
      listReports(reportPage);
    }
  }, [tab, reportPage, pub, self?.is_admin, listReports]);

  useEffect(() => {
    if (pub && self?.is_admin && tab === "review") {
      listPendingReview(pendingReviewPage);
    }
  }, [tab, pendingReviewPage, pub, self?.is_admin, listPendingReview]);

  useEffect(() => {
    if (pub && self?.is_admin && tab === "config") {
      listConfig();
    }
  }, [tab, pub, self?.is_admin, listConfig]);

  if (loading) {
    return (
      <div className="flex justify-center items-center h-48">
        <div className="text-sm text-neutral-500">Loading...</div>
      </div>
    );
  }

  if (!login) {
    return (
      <div className="max-w-sm mx-auto bg-neutral-900 border border-neutral-800 rounded-sm p-4">
        <h2 className="text-sm font-medium mb-2 text-white">
          Authentication Required
        </h2>
        <p className="text-neutral-500 text-xs">
          Please log in to access the admin panel.
        </p>
      </div>
    );
  }

  if (self && !self.is_admin) {
    return <Navigate to="/" replace />;
  }

  const tabs: { id: Tab; label: string }[] = [
    { id: "files", label: "Files" },
    { id: "reports", label: "Reports" },
    { id: "review", label: "Review" },
    { id: "config", label: "Config" },
    { id: "labeling", label: "Labeling" },
    { id: "progress", label: "Progress" },
    { id: "stats", label: "Stats" },
  ];

  return (
    <div className="space-y-4">
      <h1 className="text-xl font-medium text-white">Admin</h1>

      {error && (
        <div className="bg-red-950 border border-red-900 text-red-200 px-3 py-2 rounded-sm text-sm">
          {error}
        </div>
      )}

      <div className="flex border-b border-neutral-800">
        {tabs.map((t) => (
          <button
            key={t.id}
            onClick={() => setTab(t.id)}
            className={classNames(
              "px-4 py-2 text-sm transition-colors border-b-2 -mb-px",
              tab === t.id
                ? "border-white text-white"
                : "border-transparent text-neutral-500 hover:text-neutral-300",
            )}
          >
            {t.label}
          </button>
        ))}
      </div>

      {tab === "files" && (
        <div className="space-y-3">
          <FileListControls
            mimeFilter={mimeFilter}
            onMimeFilter={setMimeFilter}
            labelFilter={labelFilter}
            onLabelFilter={setLabelFilter}
            sortBy={sortBy}
            onSortBy={setSortBy}
            sortOrder={sortOrder}
            onSortOrder={setSortOrder}
          />
          {adminListedFiles && (
            <FileList
              files={adminListedFiles.files}
              pages={Math.ceil(adminListedFiles.total / adminListedFiles.count)}
              page={adminListedFiles.page}
              onPage={(x) => setAdminListedPage(x)}
              onDelete={async (x) => {
                await deleteFile(x);
                await listAllUploads(adminListedPage);
              }}
              onLabelClick={(l) => setLabelFilter(l)}
              onFindSimilar={findSimilar}
              adminMode={true}
            />
          )}
        </div>
      )}

      {tab === "reports" && (
        <>
          {reports && (
            <ReportList
              reports={reports}
              pages={reportPages}
              page={reportPage}
              onPage={(x) => setReportPage(x)}
              onAcknowledge={acknowledgeReport}
              onDeleteFile={async (fileId) => {
                await deleteFile(fileId);
                await listReports(reportPage);
              }}
            />
          )}
        </>
      )}

      {tab === "review" && (
        <>
          {pendingReview && (
            <>
              <div className="flex items-center justify-between">
                <span className="text-xs text-neutral-500">
                  {pendingReview.total} files
                </span>
                <div className="flex gap-1">
                  {bulkProgress ? (
                    <span className="text-xs text-neutral-500">
                      {bulkProgress}
                    </span>
                  ) : (
                    <>
                      <button
                        onClick={bulkApproveAll}
                        className="bg-neutral-800 hover:bg-neutral-700 text-white px-2 py-1 rounded-sm text-xs"
                      >
                        Approve All
                      </button>
                      <button
                        onClick={bulkBanAll}
                        className="bg-neutral-800 hover:bg-neutral-700 text-white px-2 py-1 rounded-sm text-xs"
                      >
                        Ban All
                      </button>
                    </>
                  )}
                </div>
              </div>
              <FileList
                files={pendingReview.files}
                pages={Math.ceil(pendingReview.total / pendingReview.count)}
                page={pendingReview.page}
                onPage={(x) => setPendingReviewPage(x)}
                onReview={reviewFile}
                onDelete={reviewAndDeleteFile}
                onBan={banFile}
                onFindSimilar={findSimilar}
                adminMode={true}
              />
            </>
          )}
        </>
      )}

      {tab === "config" && config && pub && (
        <ConfigEditor
          config={config}
          pub={pub}
          url={url}
          onSave={saveConfig}
          onDelete={deleteConfig}
        />
      )}

      {tab === "labeling" && pub && (
        <LabelManagement pub={pub} url={url} />
      )}

      {tab === "progress" && pub && <BackgroundProgress pub={pub} url={url} />}

      {tab === "stats" && pub && <Stats pub={pub} url={url} />}

      {(similarFiles || similarLoading) && similarSource && (
        <div
          className="fixed inset-0 z-50 flex items-center justify-center bg-black/70"
          onClick={closeSimilarModal}
        >
          <div
            className="bg-neutral-900 border border-neutral-800 rounded-sm max-w-4xl w-full mx-4 max-h-[80vh] overflow-y-auto"
            onClick={(e) => e.stopPropagation()}
          >
            <div className="flex items-center justify-between px-4 py-3 border-b border-neutral-800">
              <h2 className="text-sm font-medium text-white">Similar Images</h2>
              <button
                onClick={closeSimilarModal}
                className="text-neutral-500 hover:text-white text-sm"
              >
                Close
              </button>
            </div>
            <div className="p-4 space-y-4">
              <div className="flex items-start gap-3">
                <div className="w-24 h-24 shrink-0 rounded-sm overflow-hidden bg-neutral-950 border border-neutral-700">
                  <img
                    src={similarSource.url.replace(
                      `/${similarSource.id}`,
                      `/thumb/${similarSource.id}`,
                    )}
                    className="w-full h-full object-contain object-center"
                  />
                </div>
                <div className="text-xs space-y-1 min-w-0">
                  <div className="text-neutral-300 font-medium truncate">
                    {similarSource.name || "Untitled"}
                  </div>
                  <div className="text-neutral-500">
                    {similarSource.dim && <span>{similarSource.dim}</span>}
                    {similarSource.dim && similarSource.type && " | "}
                    {similarSource.type}
                    {similarSource.size
                      ? ` | ${FormatBytes(similarSource.size, 2)}`
                      : ""}
                  </div>
                  <div className="text-neutral-600 font-mono truncate">
                    {similarSource.id}
                  </div>
                </div>
              </div>

              {similarLoading && (
                <div className="text-sm text-neutral-500 text-center py-8">
                  Searching for similar images...
                </div>
              )}
              {similarFiles && similarFiles.length === 0 && (
                <div className="text-sm text-neutral-500 text-center py-8">
                  No similar images found.
                </div>
              )}
              {similarFiles && similarFiles.length > 0 && (
                <>
                  <div className="text-xs text-neutral-500">
                    {similarFiles.length} similar{" "}
                    {similarFiles.length === 1 ? "image" : "images"} found
                  </div>
                  <div className="grid grid-cols-2 sm:grid-cols-3 md:grid-cols-4 gap-2">
                    {similarFiles.map((f, idx) => {
                      const fileId = f.tags.find((t) => t[0] === "x")?.[1];
                      const fileUrl = f.tags.find((t) => t[0] === "url")?.[1];
                      const thumbUrl = fileUrl?.replace(
                        `/${fileId}`,
                        `/thumb/${fileId}`,
                      );
                      const mime = f.tags.find((t) => t[0] === "m")?.[1];
                      const dim = f.tags.find((t) => t[0] === "dim")?.[1];
                      const size = Number(
                        f.tags.find((t) => t[0] === "size")?.[1],
                      );
                      return (
                        <div
                          key={`${fileId}-${idx}`}
                          className="group relative rounded-sm aspect-square overflow-hidden bg-neutral-950 border border-neutral-800"
                        >
                          <img
                            src={thumbUrl}
                            className="w-full h-full object-contain object-center"
                            loading="lazy"
                          />
                          <div className="absolute inset-x-0 bottom-0 bg-black/80 px-2 py-1.5 text-xs space-y-0.5">
                            <div className="flex justify-between text-neutral-300">
                              <span>Distance: {f.distance}</span>
                              <span>
                                {size && !isNaN(size)
                                  ? FormatBytes(size, 2)
                                  : ""}
                              </span>
                            </div>
                            <div className="text-neutral-500 truncate">
                              {dim && <span>{dim}</span>}
                              {dim && mime && <span className="mx-1">|</span>}
                              {mime && <span>{mime}</span>}
                            </div>
                            <div className="flex gap-1 mt-1">
                              <a
                                href={fileUrl}
                                target="_blank"
                                className="bg-neutral-800 hover:bg-neutral-700 text-white px-2 py-0.5 rounded-sm text-xs"
                              >
                                View
                              </a>
                              <button
                                onClick={async () => {
                                  if (fileId) {
                                    await deleteFile(fileId);
                                    setSimilarFiles((prev) =>
                                      prev?.filter(
                                        (s) =>
                                          s.tags.find(
                                            (t) => t[0] === "x",
                                          )?.[1] !== fileId,
                                      ),
                                    );
                                  }
                                }}
                                className="bg-neutral-800 hover:bg-neutral-700 text-white px-2 py-0.5 rounded-sm text-xs"
                              >
                                Delete
                              </button>
                            </div>
                          </div>
                        </div>
                      );
                    })}
                  </div>
                </>
              )}
            </div>
          </div>
        </div>
      )}
    </div>
  );
}
