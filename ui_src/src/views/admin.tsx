import { useEffect, useState, useCallback } from "react";
import { Navigate } from "react-router-dom";
import Button from "../components/button";
import FileList from "./files";
import ReportList from "./reports";
import { Blossom } from "../upload/blossom";
import useLogin from "../hooks/login";
import usePublisher from "../hooks/publisher";
import { Nip96FileList } from "../upload/nip96";
import { AdminSelf, Route96, Report } from "../upload/admin";

export default function Admin() {
  const [self, setSelf] = useState<AdminSelf>();
  const [error, setError] = useState<string>();
  const [adminListedFiles, setAdminListedFiles] = useState<Nip96FileList>();
  const [reports, setReports] = useState<Report[]>();
  const [reportPages, setReportPages] = useState<number>();
  const [reportPage, setReportPage] = useState(0);
  const [adminListedPage, setAdminListedPage] = useState(0);
  const [mimeFilter, setMimeFilter] = useState<string>();
  const [loading, setLoading] = useState(true);

  const login = useLogin();
  const pub = usePublisher();

  const url =
    import.meta.env.VITE_API_URL || `${location.protocol}//${location.host}`;

  const listAllUploads = useCallback(
    async (n: number) => {
      if (!pub) return;
      try {
        setError(undefined);
        const uploader = new Route96(url, pub);
        const result = await uploader.listFiles(n, 50, mimeFilter);
        setAdminListedFiles(result);
      } catch (e) {
        if (e instanceof Error) {
          setError(e.message.length > 0 ? e.message : "Upload failed");
        } else if (typeof e === "string") {
          setError(e);
        } else {
          setError("List files failed");
        }
      }
    },
    [pub, url, mimeFilter],
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
        if (e instanceof Error) {
          setError(e.message.length > 0 ? e.message : "List reports failed");
        } else if (typeof e === "string") {
          setError(e);
        } else {
          setError("List reports failed");
        }
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
      if (e instanceof Error) {
        setError(
          e.message.length > 0 ? e.message : "Acknowledge report failed",
        );
      } else if (typeof e === "string") {
        setError(e);
      } else {
        setError("Acknowledge report failed");
      }
    }
  }

  async function deleteFile(id: string) {
    if (!pub) return;
    try {
      setError(undefined);
      const uploader = new Blossom(url, pub);
      await uploader.delete(id);
    } catch (e) {
      if (e instanceof Error) {
        setError(e.message.length > 0 ? e.message : "Upload failed");
      } else if (typeof e === "string") {
        setError(e);
      } else {
        setError("List files failed");
      }
    }
  }

  useEffect(() => {
    if (pub && !self) {
      const r96 = new Route96(url, pub);
      r96
        .getSelf()
        .then((v) => {
          setSelf(v.data);
          setLoading(false);
        })
        .catch(() => {
          setLoading(false);
        });
    }
  }, [pub, self, url]);

  useEffect(() => {
    if (pub && self?.is_admin) {
      listAllUploads(adminListedPage);
    }
  }, [adminListedPage, pub, self?.is_admin, listAllUploads]);

  useEffect(() => {
    if (pub && self?.is_admin) {
      listReports(reportPage);
    }
  }, [reportPage, pub, self?.is_admin, listReports]);

  if (loading) {
    return (
      <div className="flex justify-center items-center h-64">
        <div className="text-lg text-neutral-400">Loading...</div>
      </div>
    );
  }

  if (!login) {
    return (
      <div className="max-w-md mx-auto bg-neutral-800 border border-neutral-700 rounded-lg shadow-sm">
        <div className="text-center p-6">
          <h2 className="text-xl font-semibold mb-4 text-neutral-100">Authentication Required</h2>
          <p className="text-neutral-300">
            Please log in to access the admin panel.
          </p>
        </div>
      </div>
    );
  }

  if (!self?.is_admin) {
    return <Navigate to="/" replace />;
  }

  return (
    <div className="space-y-8 px-4">
      <div className="flex items-center justify-between">
        <h1 className="text-3xl font-bold text-neutral-100">Admin Panel</h1>
      </div>

      {error && (
        <div className="bg-red-900 border border-red-700 text-red-200 px-4 py-3 rounded">
          {error}
        </div>
      )}

      <div className="grid gap-8 lg:grid-cols-2">
        <div className="bg-neutral-800 border border-neutral-700 rounded-lg shadow-sm">
          <div className="p-6">
            <h3 className="text-lg font-semibold mb-4 text-neutral-100">File Management</h3>
            <div className="space-y-4">
              <div>
                <label className="block text-sm font-medium mb-2 text-neutral-300">
                  Filter by MIME type
                </label>
                <select
                  className="flex h-9 w-full rounded-md border border-neutral-600 bg-neutral-700 px-3 py-1 text-sm text-neutral-100 shadow-sm transition-colors focus:outline-none focus:ring-1 focus:ring-neutral-500 disabled:cursor-not-allowed disabled:opacity-50"
                  value={mimeFilter || ""}
                  onChange={(e) => setMimeFilter(e.target.value || undefined)}
                >
                  <option value="">All Files</option>
                  <option value="image/webp">WebP Images</option>
                  <option value="image/jpeg">JPEG Images</option>
                  <option value="image/jpg">JPG Images</option>
                  <option value="image/png">PNG Images</option>
                  <option value="image/gif">GIF Images</option>
                  <option value="video/mp4">MP4 Videos</option>
                  <option value="video/mov">MOV Videos</option>
                </select>
              </div>

              <Button
                onClick={() => listAllUploads(0)}
                className="w-full"
              >
                Load All Files
              </Button>
            </div>
          </div>
        </div>

        <div className="bg-neutral-800 border border-neutral-700 rounded-lg shadow-sm">
          <div className="p-6">
            <h3 className="text-lg font-semibold mb-4 text-neutral-100">Reports Management</h3>
            <Button onClick={() => listReports(0)} className="w-full">
              Load Reports
            </Button>
          </div>
        </div>
      </div>

      {adminListedFiles && (
        <div className="bg-neutral-800 border border-neutral-700 rounded-lg shadow-sm">
          <div className="p-6">
            <h3 className="text-lg font-semibold mb-4 text-neutral-100">All Files</h3>
            <FileList
              files={adminListedFiles.files}
              pages={Math.ceil(adminListedFiles.total / adminListedFiles.count)}
              page={adminListedFiles.page}
              onPage={(x) => setAdminListedPage(x)}
              onDelete={async (x) => {
                await deleteFile(x);
                await listAllUploads(adminListedPage);
              }}
              adminMode={true}
            />
          </div>
        </div>
      )}

      {reports && (
        <div className="bg-neutral-800 border border-neutral-700 rounded-lg shadow-sm">
          <div className="p-6">
            <h3 className="text-lg font-semibold mb-4 text-neutral-100">Reports</h3>
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
          </div>
        </div>
      )}
    </div>
  );
}
