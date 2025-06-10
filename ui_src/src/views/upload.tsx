import { useEffect, useState } from "react";
import Button from "../components/button";
import FileList from "./files";
import { openFile } from "../upload";
import { Blossom } from "../upload/blossom";
import useLogin from "../hooks/login";
import usePublisher from "../hooks/publisher";
import { Nip96, Nip96FileList } from "../upload/nip96";
import { AdminSelf, Route96, Report } from "../upload/admin";
import { FormatBytes } from "../const";
import ReportJson from "../report.json";
import ReportList from "./reports";

export default function Upload() {
  const [type, setType] = useState<"blossom" | "nip96">("blossom");
  const [noCompress, setNoCompress] = useState(false);
  const [showLegacy, setShowLegacy] = useState(false);
  const [toUpload, setToUpload] = useState<File>();
  const [self, setSelf] = useState<AdminSelf>();
  const [error, setError] = useState<string>();
  const [bulkPrgress, setBulkProgress] = useState<number>();
  const [results, setResults] = useState<Array<object>>([]);
  const [listedFiles, setListedFiles] = useState<Nip96FileList>();
  const [adminListedFiles, setAdminListedFiles] = useState<Nip96FileList>();
  const [adminListedReports, setAdminListedReports] = useState<{
    total: number;
    page: number;
    count: number;
    files: Array<Report>;
  }>();
  const [listedPage, setListedPage] = useState(0);
  const [adminListedPage, setAdminListedPage] = useState(0);
  const [adminReportsPage, setAdminReportsPage] = useState(0);
  const [mimeFilter, setMimeFilter] = useState<string>();

  const login = useLogin();
  const pub = usePublisher();

  const legacyFiles = ReportJson as Record<string, Array<string>>;
  const myLegacyFiles = login ? (legacyFiles[login.pubkey] ?? []) : [];

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
      const uploader = new Route96(url, pub);
      const result = await uploader.listReports(n, 50);
      setAdminListedReports(result);
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
      const uploader = new Route96(url, pub);
      await uploader.acknowledgeReport(reportId);
      // Refresh the reports list
      await listReports(adminReportsPage);
    } catch (e) {
      if (e instanceof Error) {
        setError(e.message.length > 0 ? e.message : "Acknowledge failed");
      } else if (typeof e === "string") {
        setError(e);
      } else {
        setError("Acknowledge failed");
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

  async function migrateLegacy() {
    if (!pub) return;
    const uploader = new Blossom(url, pub);
    let ctr = 0;
    for (const f of myLegacyFiles) {
      try {
        await uploader.mirror(`https://void.cat/d/${f}`);
      } catch (e) {
        console.error(e);
      }
      setBulkProgress(ctr++ / myLegacyFiles.length);
    }
  }

  useEffect(() => {
    listUploads(listedPage);
  }, [listedPage]);

  useEffect(() => {
    listAllUploads(adminListedPage);
  }, [adminListedPage, mimeFilter]);

  useEffect(() => {
    listReports(adminReportsPage);
  }, [adminReportsPage]);

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

      {login && myLegacyFiles.length > 0 && (
        <div className="flex flex-col gap-4 font-bold">
          You have {myLegacyFiles.length.toLocaleString()} files which can be
          migrated from void.cat
          <div className="flex gap-2">
            <Button onClick={() => migrateLegacy()}>Migrate Files</Button>
            <Button onClick={() => setShowLegacy((s) => !s)}>
              {!showLegacy ? "Show Files" : "Hide Files"}
            </Button>
          </div>
          {bulkPrgress !== undefined && <progress value={bulkPrgress} />}
        </div>
      )}
      {showLegacy && (
        <FileList
          files={myLegacyFiles.map((f) => ({
            id: f,
            url: `https://void.cat/d/${f}`,
          }))}
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
          
          <hr />
          <h3>Admin Reports:</h3>
          <Button onClick={() => listReports(0)}>List Reports</Button>
          {adminListedReports && (
            <ReportList
              reports={adminListedReports.files}
              pages={Math.ceil(adminListedReports.total / adminListedReports.count)}
              page={adminListedReports.page}
              onPage={(x) => setAdminReportsPage(x)}
              onAcknowledge={acknowledgeReport}
              onDeleteFile={async (fileId) => {
                await deleteFile(fileId);
                await listReports(adminReportsPage);
              }}
            />
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
