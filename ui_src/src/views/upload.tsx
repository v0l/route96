import { useEffect, useState } from "react";
import Button from "../components/button";
import FileList from "./files";
import ReportList from "./reports";
import PaymentFlow from "../components/payment";
import { openFile } from "../upload";
import { Blossom } from "../upload/blossom";
import useLogin from "../hooks/login";
import usePublisher from "../hooks/publisher";
import { Nip96, Nip96FileList } from "../upload/nip96";
import { AdminSelf, Route96, Report } from "../upload/admin";
import { FormatBytes } from "../const";

export default function Upload() {
  const [type, setType] = useState<"blossom" | "nip96">("blossom");
  const [noCompress, setNoCompress] = useState(false);
  const [toUpload, setToUpload] = useState<File>();
  const [self, setSelf] = useState<AdminSelf>();
  const [error, setError] = useState<string>();
  const [results, setResults] = useState<Array<object>>([]);
  const [listedFiles, setListedFiles] = useState<Nip96FileList>();
  const [adminListedFiles, setAdminListedFiles] = useState<Nip96FileList>();
  const [reports, setReports] = useState<Report[]>();
  const [reportPages, setReportPages] = useState<number>();
  const [reportPage, setReportPage] = useState(0);
  const [listedPage, setListedPage] = useState(0);
  const [adminListedPage, setAdminListedPage] = useState(0);
  const [mimeFilter, setMimeFilter] = useState<string>();
  const [showPaymentFlow, setShowPaymentFlow] = useState(false);

  const login = useLogin();
  const pub = usePublisher();

  const url =
    import.meta.env.VITE_API_URL || `${location.protocol}//${location.host}`;
  async function doUpload() {
    if (!pub) return;
    if (!toUpload) return;
    try {
      setError(undefined);
      if (type === "blossom") {
        const uploader = new Blossom(url, pub);
        const result = noCompress
          ? await uploader.upload(toUpload)
          : await uploader.media(toUpload);
        setResults((s) => [...s, result]);
      }
      if (type === "nip96") {
        const uploader = new Nip96(url, pub);
        await uploader.loadInfo();
        const result = await uploader.upload(toUpload);
        setResults((s) => [...s, result]);
      }
    } catch (e) {
      if (e instanceof Error) {
        setError(e.message.length > 0 ? e.message : "Upload failed");
      } else if (typeof e === "string") {
        setError(e);
      } else {
        setError("Upload failed");
      }
    }
  }

  async function listUploads(n: number) {
    if (!pub) return;
    try {
      setError(undefined);
      const uploader = new Nip96(url, pub);
      await uploader.loadInfo();
      const result = await uploader.listFiles(n, 50);
      setListedFiles(result);
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

  async function listAllUploads(n: number) {
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
  }

  async function listReports(n: number) {
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
  }

  async function acknowledgeReport(reportId: number) {
    if (!pub) return;
    try {
      setError(undefined);
      const route96 = new Route96(url, pub);
      await route96.acknowledgeReport(reportId);
      await listReports(reportPage); // Refresh the list
    } catch (e) {
      if (e instanceof Error) {
        setError(e.message.length > 0 ? e.message : "Acknowledge report failed");
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
      listUploads(listedPage);
    }
  }, [listedPage, pub]);

  useEffect(() => {
    if (pub) {
      listAllUploads(adminListedPage);
    }
  }, [adminListedPage, mimeFilter, pub]);

  useEffect(() => {
    if (pub && self?.is_admin) {
      listReports(reportPage);
    }
  }, [reportPage, pub, self?.is_admin]);

  useEffect(() => {
    if (pub && !self) {
      const r96 = new Route96(url, pub);
      r96.getSelf().then((v) => setSelf(v.data));
    }
  }, [pub, self]);

  return (
    <div className="flex flex-col gap-2 bg-neutral-800 p-8 rounded-xl w-full">
      <h1 className="text-lg font-bold">
        Welcome to {window.location.hostname}
      </h1>
      <div className="text-neutral-400 uppercase text-xs font-medium">
        Upload Method
      </div>
      <div className="flex gap-4 items-center">
        <div
          className="flex gap-2 cursor-pointer"
          onClick={() => setType("blossom")}
        >
          Blossom
          <input type="radio" checked={type === "blossom"} />
        </div>
        <div
          className="flex gap-2 cursor-pointer"
          onClick={() => setType("nip96")}
        >
          NIP-96
          <input type="radio" checked={type === "nip96"} />
        </div>
      </div>

      <div
        className="flex gap-2 cursor-pointer"
        onClick={() => setNoCompress((s) => !s)}
      >
        Disable Compression
        <input type="checkbox" checked={noCompress} />
      </div>

      {toUpload && <FileList files={toUpload ? [toUpload] : []} />}
      <div className="flex gap-4">
        <Button
          className="flex-1"
          onClick={async () => {
            const f = await openFile();
            setToUpload(f);
          }}
        >
          Choose Files
        </Button>
        <Button
          className="flex-1"
          onClick={doUpload}
          disabled={login === undefined}
        >
          Upload
        </Button>
      </div>
      <hr />
      {!listedFiles && (
        <Button disabled={login === undefined} onClick={() => listUploads(0)}>
          List Uploads
        </Button>
      )}

      {self && (
        <div className="flex justify-between font-medium">
          <div>Uploads: {self.file_count.toLocaleString()}</div>
          <div>Total Size: {FormatBytes(self.total_size)}</div>
        </div>
      )}

      {self && (
        <div className="bg-neutral-700 p-4 rounded-lg">
          <h3 className="text-lg font-bold mb-2">Storage Quota</h3>
          <div className="space-y-2">
            {self.free_quota && (
              <div className="text-sm">
                Free Quota: {FormatBytes(self.free_quota)}
              </div>
            )}
            {self.quota && (
              <div className="text-sm">
                Paid Quota: {FormatBytes(self.quota)}
              </div>
            )}
            {self.total_available_quota && (
              <div className="text-sm font-medium">
                Total Available: {FormatBytes(self.total_available_quota)}
              </div>
            )}
            {self.total_available_quota && (
              <div className="text-sm">
                Remaining: {FormatBytes(Math.max(0, self.total_available_quota - self.total_size))}
              </div>
            )}
            {self.paid_until && (
              <div className="text-sm text-neutral-300">
                Paid Until: {new Date(self.paid_until * 1000).toLocaleDateString()}
              </div>
            )}
          </div>
          <Button 
            onClick={() => setShowPaymentFlow(!showPaymentFlow)} 
            className="mt-3 w-full"
          >
            {showPaymentFlow ? "Hide" : "Show"} Top Up Options
          </Button>
        </div>
      )}

      {showPaymentFlow && pub && (
        <PaymentFlow 
          route96={new Route96(url, pub)} 
          onPaymentRequested={(pr) => {
            console.log("Payment requested:", pr);
            // You could add more logic here, like showing a QR code
          }}
        />
      )}

      {listedFiles && (
        <FileList
          files={listedFiles.files}
          pages={Math.ceil(listedFiles.total / listedFiles.count)}
          page={listedFiles.page}
          onPage={(x) => setListedPage(x)}
          onDelete={async (x) => {
            await deleteFile(x);
            await listUploads(listedPage);
          }}
        />
      )}

      {self?.is_admin && (
        <>
          <hr />
          <h3>Admin File List:</h3>
          <Button onClick={() => listAllUploads(0)}>List All Uploads</Button>
          <Button onClick={() => listReports(0)}>List Reports</Button>
          <div>
            <select value={mimeFilter} onChange={e => setMimeFilter(e.target.value)}>
              <option value={""}>All</option>
              <option>image/webp</option>
              <option>image/jpeg</option>
              <option>image/jpg</option>
              <option>image/png</option>
              <option>image/gif</option>
              <option>video/mp4</option>
              <option>video/mov</option>
            </select>
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
            />
          )}
          {reports && (
            <>
              <h3>Reports:</h3>
              <ReportList
                reports={reports}
                pages={reportPages}
                page={reportPage}
                onPage={(x) => setReportPage(x)}
                onAcknowledge={acknowledgeReport}
                onDeleteFile={async (fileId) => {
                  await deleteFile(fileId);
                  await listReports(reportPage); // Refresh reports after deleting file
                }}
              />
            </>
          )}
        </>
      )}
      {error && <b className="text-red-500">{error}</b>}
      <pre className="text-xs font-monospace overflow-wrap">
        {JSON.stringify(results, undefined, 2)}
      </pre>
    </div>
  );
}
