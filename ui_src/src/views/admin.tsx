import { useEffect, useState, useCallback } from "react";
import { Navigate } from "react-router-dom";
import classNames from "classnames";
import FileList from "./files";
import ReportList from "./reports";
import useLogin from "../hooks/login";
import usePublisher from "../hooks/publisher";
import { Nip96FileList } from "../upload/nip96";
import { AdminSelf, Route96, Report } from "../upload/admin";
import { Blossom } from "../upload/blossom";

type Tab = "files" | "reports" | "review";

export default function Admin() {
  const [self, setSelf] = useState<AdminSelf>();
  const [error, setError] = useState<string>();
  const [tab, setTab] = useState<Tab>("files");
  const [loading, setLoading] = useState(true);

  // Files tab
  const [adminListedFiles, setAdminListedFiles] = useState<Nip96FileList>();
  const [adminListedPage, setAdminListedPage] = useState(0);
  const [mimeFilter, setMimeFilter] = useState<string>();
  const [labelFilter, setLabelFilter] = useState<string>();
  const [bulkProgress, setBulkProgress] = useState<string>();

  // Reports tab
  const [reports, setReports] = useState<Report[]>();
  const [reportPages, setReportPages] = useState<number>();
  const [reportPage, setReportPage] = useState(0);

  // Review tab
  const [pendingReview, setPendingReview] = useState<Nip96FileList>();
  const [pendingReviewPage, setPendingReviewPage] = useState(0);

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
        const result = await route96.listFiles(n, 50, mimeFilter, labelFilter);
        setAdminListedFiles(result);
      } catch (e) {
        setError(
          e instanceof Error
            ? e.message || "List files failed"
            : "List files failed",
        );
      }
    },
    [pub, url, mimeFilter, labelFilter],
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

  async function acknowledgeReport(reportId: number) {
    if (!pub) return;
    try {
      setError(undefined);
      const route96 = new Route96(url, pub);
      await route96.acknowledgeReport(reportId);
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
      await route96.reviewFile(id);
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
      await route96.deleteReviewFile(id);
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
      await route96.reviewFile(id);
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
    const files = pendingReview.files;
    const route96 = new Route96(url, pub);
    for (let i = 0; i < files.length; i++) {
      const id = files[i].tags.find((t) => t[0] === "x")?.[1];
      if (!id) continue;
      setBulkProgress(`Approving ${i + 1} / ${files.length}...`);
      try {
        await route96.reviewFile(id);
      } catch {
        // continue on individual failures
      }
    }
    setBulkProgress(undefined);
    await listPendingReview(pendingReviewPage);
  }

  async function bulkBanAll() {
    if (!pub || !pendingReview) return;
    const files = pendingReview.files;
    const route96 = new Route96(url, pub);
    for (let i = 0; i < files.length; i++) {
      const id = files[i].tags.find((t) => t[0] === "x")?.[1];
      if (!id) continue;
      setBulkProgress(`Banning ${i + 1} / ${files.length}...`);
      try {
        await route96.deleteReviewFile(id);
      } catch {
        // continue on individual failures
      }
    }
    setBulkProgress(undefined);
    await listPendingReview(pendingReviewPage);
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
          <div className="flex items-center gap-2">
            <select
              className="h-7 rounded-sm border border-neutral-800 bg-neutral-950 px-2 text-xs text-neutral-300"
              value={mimeFilter || ""}
              onChange={(e) => setMimeFilter(e.target.value || undefined)}
            >
              <option value="">All types</option>
              <option value="image/webp">WebP</option>
              <option value="image/jpeg">JPEG</option>
              <option value="image/png">PNG</option>
              <option value="image/gif">GIF</option>
              <option value="video/mp4">MP4</option>
              <option value="video/mov">MOV</option>
            </select>
            <input
              type="text"
              placeholder="Filter by label..."
              className="h-7 rounded-sm border border-neutral-800 bg-neutral-950 px-2 text-xs text-neutral-300 placeholder-neutral-600"
              value={labelFilter || ""}
              onChange={(e) => setLabelFilter(e.target.value || undefined)}
            />
          </div>
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
                adminMode={true}
              />
            </>
          )}
        </>
      )}
    </div>
  );
}
