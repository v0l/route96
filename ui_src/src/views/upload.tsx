import { useEffect, useState, useCallback } from "react";
import Button from "../components/button";
import FileList from "./files";
import PaymentFlow from "../components/payment";
import { openFile } from "../upload";
import { Blossom } from "../upload/blossom";
import useLogin from "../hooks/login";
import usePublisher from "../hooks/publisher";
import { Nip96, Nip96FileList } from "../upload/nip96";
import { AdminSelf, Route96 } from "../upload/admin";
import { FormatBytes } from "../const";

export default function Upload() {
  const [type, setType] = useState<"blossom" | "nip96">("blossom");
  const [noCompress, setNoCompress] = useState(false);
  const [toUpload, setToUpload] = useState<File>();
  const [self, setSelf] = useState<AdminSelf>();
  const [error, setError] = useState<string>();
  const [results, setResults] = useState<Array<object>>([]);
  const [listedFiles, setListedFiles] = useState<Nip96FileList>();
  const [listedPage, setListedPage] = useState(0);
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

  const listUploads = useCallback(async (n: number) => {
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
  }, [pub, url]);


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
    if (pub && !listedFiles) {
      listUploads(listedPage);
    }
  }, [listedPage, pub, listUploads, listedFiles]);


  useEffect(() => {
    if (pub && !self) {
      const r96 = new Route96(url, pub);
      r96.getSelf().then((v) => setSelf(v.data));
    }
  }, [pub, self, url]);

  if (!login) {
    return (
      <div className="card max-w-2xl mx-auto text-center">
        <h2 className="text-2xl font-semibold mb-4 text-gray-100">Welcome to {window.location.hostname}</h2>
        <p className="text-gray-400 mb-6">Please log in to start uploading files to your storage.</p>
      </div>
    );
  }

  return (
    <div className="max-w-4xl mx-auto space-y-8">
      <div className="text-center">
        <h1 className="text-3xl font-bold text-gray-100 mb-2">
          Welcome to {window.location.hostname}
        </h1>
        <p className="text-lg text-gray-400">Upload and manage your files securely</p>
      </div>

      {error && (
        <div className="bg-red-900/20 border border-red-800 text-red-400 px-4 py-3 rounded-lg">
          {error}
        </div>
      )}

      <div className="card">
        <h2 className="text-xl font-semibold mb-6">Upload Settings</h2>
        
        <div className="space-y-6">
          <div>
            <label className="block text-sm font-medium text-gray-300 mb-3">
              Upload Method
            </label>
            <div className="flex gap-6">
              <label className="flex items-center cursor-pointer">
                <input
                  type="radio"
                  checked={type === "blossom"}
                  onChange={() => setType("blossom")}
                  className="mr-2"
                />
                <span className="text-sm font-medium text-gray-300">Blossom</span>
              </label>
              <label className="flex items-center cursor-pointer">
                <input
                  type="radio"
                  checked={type === "nip96"}
                  onChange={() => setType("nip96")}
                  className="mr-2"
                />
                <span className="text-sm font-medium text-gray-300">NIP-96</span>
              </label>
            </div>
          </div>

          <div>
            <label className="flex items-center cursor-pointer">
              <input
                type="checkbox"
                checked={noCompress}
                onChange={(e) => setNoCompress(e.target.checked)}
                className="mr-2"
              />
              <span className="text-sm font-medium text-gray-300">Disable Compression</span>
            </label>
          </div>

          {toUpload && (
            <div className="border-2 border-dashed border-gray-600 rounded-lg p-4">
              <FileList files={[toUpload]} />
            </div>
          )}

          <div className="flex gap-4">
            <Button
              onClick={async () => {
                const f = await openFile();
                setToUpload(f);
              }}
              className="btn-secondary flex-1"
            >
              Choose File
            </Button>
            <Button
              onClick={doUpload}
              disabled={!toUpload}
              className="btn-primary flex-1"
            >
              Upload
            </Button>
          </div>
        </div>
      </div>

      {self && (
        <div className="grid gap-6 md:grid-cols-2">
          <div className="card">
            <h3 className="text-lg font-semibold mb-4">Storage Usage</h3>
            <div className="space-y-2">
              <div className="flex justify-between text-sm">
                <span>Files:</span>
                <span className="font-medium">{self.file_count.toLocaleString()}</span>
              </div>
              <div className="flex justify-between text-sm">
                <span>Total Size:</span>
                <span className="font-medium">{FormatBytes(self.total_size)}</span>
              </div>
            </div>
          </div>

          <div className="card">
            <h3 className="text-lg font-semibold mb-4">Storage Quota</h3>
            <div className="space-y-2">
              {self.free_quota && (
                <div className="flex justify-between text-sm">
                  <span>Free Quota:</span>
                  <span className="font-medium">{FormatBytes(self.free_quota)}</span>
                </div>
              )}
              {self.quota && (
                <div className="flex justify-between text-sm">
                  <span>Paid Quota:</span>
                  <span className="font-medium">{FormatBytes(self.quota)}</span>
                </div>
              )}
              {self.total_available_quota && (
                <div className="flex justify-between text-sm font-medium">
                  <span>Total Available:</span>
                  <span>{FormatBytes(self.total_available_quota)}</span>
                </div>
              )}
              {self.total_available_quota && (
                <div className="flex justify-between text-sm">
                  <span>Remaining:</span>
                  <span className="font-medium text-green-400">
                    {FormatBytes(Math.max(0, self.total_available_quota - self.total_size))}
                  </span>
                </div>
              )}
              {self.paid_until && (
                <div className="flex justify-between text-sm text-gray-400">
                  <span>Paid Until:</span>
                  <span>{new Date(self.paid_until * 1000).toLocaleDateString()}</span>
                </div>
              )}
            </div>
            <Button 
              onClick={() => setShowPaymentFlow(!showPaymentFlow)} 
              className="btn-primary w-full mt-4"
            >
              {showPaymentFlow ? "Hide" : "Show"} Payment Options
            </Button>
          </div>
        </div>
      )}

      {showPaymentFlow && pub && (
        <div className="card">
          <PaymentFlow 
            route96={new Route96(url, pub)} 
            onPaymentRequested={(pr) => {
              console.log("Payment requested:", pr);
            }}
          />
        </div>
      )}

      <div className="card">
        <div className="flex justify-between items-center mb-6">
          <h2 className="text-xl font-semibold">Your Files</h2>
          {!listedFiles && (
            <Button onClick={() => listUploads(0)} className="btn-primary">
              Load Files
            </Button>
          )}
        </div>
        
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
      </div>

      {results.length > 0 && (
        <div className="card">
          <h3 className="text-lg font-semibold mb-4">Upload Results</h3>
          <pre className="text-xs bg-gray-100 p-4 rounded overflow-auto">
            {JSON.stringify(results, undefined, 2)}
          </pre>
        </div>
      )}
    </div>
  );
}
