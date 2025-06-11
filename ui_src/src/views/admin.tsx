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
    if (pub && self?.is_admin && !adminListedFiles) {
      listAllUploads(adminListedPage);
    }
  }, [adminListedPage, pub, self?.is_admin, listAllUploads, adminListedFiles]);

  useEffect(() => {
    if (pub && self?.is_admin && !reports) {
      listReports(reportPage);
    }
  }, [reportPage, pub, self?.is_admin, listReports, reports]);

  if (loading) {
    return (
      <div className="flex justify-center items-center h-64">
        <div className="text-lg text-gray-400">Loading...</div>
      </div>
    );
  }

  if (!login) {
    return (
      <div className="card max-w-md mx-auto text-center">
        <h2 className="text-xl font-semibold mb-4">Authentication Required</h2>
        <p className="text-gray-400">
          Please log in to access the admin panel.
        </p>
      </div>
    );
  }

  if (!self?.is_admin) {
    return <Navigate to="/" replace />;
  }

  return (
    <div className="space-y-8">
      <div className="flex items-center justify-between">
        <h1 className="text-3xl font-bold text-gray-100">Admin Panel</h1>
      </div>

      {error && (
        <div className="bg-red-900/20 border border-red-800 text-red-400 px-4 py-3 rounded-lg">
          {error}
        </div>
      )}

      <div className="grid gap-8 lg:grid-cols-2">
        <div className="card">
          <h2 className="text-xl font-semibold mb-6">File Management</h2>

          <div className="space-y-4">
            <div>
              <label className="block text-sm font-medium text-gray-300 mb-2">
                Filter by MIME type
              </label>
              <select
                className="input w-full"
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
              className="btn-primary w-full"
            >
              Load All Files
            </Button>
          </div>
        </div>

        <div className="card">
          <h2 className="text-xl font-semibold mb-6">Reports Management</h2>

          <Button onClick={() => listReports(0)} className="btn-primary w-full">
            Load Reports
          </Button>
        </div>
      </div>

      {adminListedFiles && (
        <div className="card">
          <h2 className="text-xl font-semibold mb-6">All Files</h2>
          <FileList
            files={adminListedFiles.files}
            pages={Math.ceil(adminListedFiles.total / adminListedFiles.count)}
            page={adminListedFiles.page}
            onPage={(x) => setAdminListedPage(x)}
            onDelete={async (x) => {
              await deleteFile(x);
              await listAllUploads(adminListedPage);
            }}
          />
        </div>
      )}

      {reports && (
        <div className="card">
          <h2 className="text-xl font-semibold mb-6">Reports</h2>
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
      )}
    </div>
  );
}
