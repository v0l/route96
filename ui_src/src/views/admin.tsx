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
    if (pub) {
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
  }, [login, pub, url]);

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
      <div className="flex justify-center items-center h-48">
        <div className="text-sm text-neutral-500">Loading...</div>
      </div>
    );
  }

  if (!login) {
    return (
      <div className="max-w-sm mx-auto bg-neutral-900 border border-neutral-800 rounded-sm p-4">
        <h2 className="text-sm font-medium mb-2 text-white">Authentication Required</h2>
        <p className="text-neutral-500 text-xs">
          Please log in to access the admin panel.
        </p>
      </div>
    );
  }

  if (self && !self.is_admin) {
    return <Navigate to="/" replace />;
  }

  return (
    <div className="space-y-4">
      <h1 className="text-xl font-medium text-white">Admin</h1>

      {error && (
        <div className="bg-red-950 border border-red-900 text-red-200 px-3 py-2 rounded-sm text-sm">
          {error}
        </div>
      )}

      <div className="grid gap-4 lg:grid-cols-2">
        <div className="bg-neutral-900 border border-neutral-800 rounded-sm p-3">
          <h3 className="text-sm font-medium mb-3 text-white">Files</h3>
          <div className="space-y-3">
            <div>
              <label className="block text-xs text-neutral-500 mb-1">
                MIME Filter
              </label>
              <select
                className="w-full h-7 rounded-sm border border-neutral-800 bg-neutral-950 px-2 text-xs text-neutral-300"
                value={mimeFilter || ""}
                onChange={(e) => setMimeFilter(e.target.value || undefined)}
              >
                <option value="">All</option>
                <option value="image/webp">WebP</option>
                <option value="image/jpeg">JPEG</option>
                <option value="image/png">PNG</option>
                <option value="image/gif">GIF</option>
                <option value="video/mp4">MP4</option>
                <option value="video/mov">MOV</option>
              </select>
            </div>

            <Button onClick={() => listAllUploads(0)} className="w-full" size="sm">
              Load Files
            </Button>
          </div>
        </div>

        <div className="bg-neutral-900 border border-neutral-800 rounded-sm p-3">
          <h3 className="text-sm font-medium mb-3 text-white">Reports</h3>
          <Button onClick={() => listReports(0)} className="w-full" size="sm">
            Load Reports
          </Button>
        </div>
      </div>

      {adminListedFiles && (
        <div className="bg-neutral-900 border border-neutral-800 rounded-sm p-3">
          <h3 className="text-sm font-medium mb-3 text-white">All Files</h3>
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
      )}

      {reports && (
        <div className="bg-neutral-900 border border-neutral-800 rounded-sm p-3">
          <h3 className="text-sm font-medium mb-3 text-white">Reports</h3>
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
