import { useEffect, useState, useCallback } from "react";
import Button from "../components/button";
import FileList from "./files";
import PaymentFlow from "../components/payment";
import ProgressBar from "../components/progress-bar";
import { openFiles } from "../upload";
import { Blossom, BlobDescriptor } from "../upload/blossom";
import useLogin from "../hooks/login";
import usePublisher from "../hooks/publisher";
import { Nip96, Nip96FileList } from "../upload/nip96";
import { AdminSelf, Route96 } from "../upload/admin";
import { FormatBytes } from "../const";
import { UploadProgress } from "../upload/progress";

export default function Upload() {
  const [noCompress, setNoCompress] = useState(false);
  const [self, setSelf] = useState<AdminSelf>();
  const [error, setError] = useState<string>();
  const [results, setResults] = useState<Array<BlobDescriptor>>([]);
  const [listedFiles, setListedFiles] = useState<Nip96FileList>();
  const [listedPage, setListedPage] = useState(0);
  const [showPaymentFlow, setShowPaymentFlow] = useState(false);
  const [isUploading, setIsUploading] = useState(false);
  const [uploadProgress, setUploadProgress] = useState<UploadProgress>();

  const login = useLogin();
  const pub = usePublisher();

  const url =
    import.meta.env.VITE_API_URL || `${location.protocol}//${location.host}`;
  
  // Check if file should have compression enabled by default
  const shouldCompress = (file: File) => {
    return file.type.startsWith('video/') || file.type.startsWith('image/');
  };

  async function doUpload(file: File) {
    if (!pub) return;
    if (!file) return;
    if (isUploading) return; // Prevent multiple uploads

    try {
      setError(undefined);
      setIsUploading(true);
      setUploadProgress(undefined);

      const onProgress = (progress: UploadProgress) => {
        setUploadProgress(progress);
      };

      const uploader = new Blossom(url, pub);
      // Use compression by default for video and image files, unless explicitly disabled
      const useCompression = shouldCompress(file) && !noCompress;
      const result = useCompression
        ? await uploader.media(file, onProgress)
        : await uploader.upload(file, onProgress);
      setResults((s) => [...s, result]);
    } catch (e) {
      if (e instanceof Error) {
        setError(e.message || "Upload failed - no error details provided");
      } else if (typeof e === "string") {
        setError(e);
      } else {
        setError("Upload failed");
      }
    } finally {
      setIsUploading(false);
      setUploadProgress(undefined);
    }
  }

  async function handleFileSelection() {
    if (isUploading) return;
    
    try {
      const files = await openFiles();
      if (!files || files.length === 0) return;
      
      // Start uploading each file immediately
      for (let i = 0; i < files.length; i++) {
        const file = files[i];
        await doUpload(file);
      }
    } catch (e) {
      if (e instanceof Error) {
        setError(e.message || "File selection failed");
      } else {
        setError("File selection failed");
      }
    }
  }

  const listUploads = useCallback(
    async (n: number) => {
      if (!pub) return;
      try {
        setError(undefined);
        const uploader = new Nip96(url, pub);
        await uploader.loadInfo();
        const result = await uploader.listFiles(n, 50);
        setListedFiles(result);
      } catch (e) {
        if (e instanceof Error) {
          setError(
            e.message || "List files failed - no error details provided",
          );
        } else if (typeof e === "string") {
          setError(e);
        } else {
          setError("List files failed");
        }
      }
    },
    [pub, url],
  );

  async function deleteFile(id: string) {
    if (!pub) return;
    try {
      setError(undefined);
      const uploader = new Blossom(url, pub);
      await uploader.delete(id);
    } catch (e) {
      if (e instanceof Error) {
        setError(e.message || "Delete failed - no error details provided");
      } else if (typeof e === "string") {
        setError(e);
      } else {
        setError("Delete failed");
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
        <h2 className="text-2xl font-semibold mb-4 text-gray-100">
          Welcome to {window.location.hostname}
        </h2>
        <p className="text-gray-400 mb-6">
          Please log in to start uploading files to your storage.
        </p>
      </div>
    );
  }

  return (
    <div className="max-w-4xl mx-auto space-y-8">
      {error && (
        <div className="bg-red-900/20 border border-red-800 text-red-400 px-4 py-3 rounded-lg">
          {error}
        </div>
      )}

      <div className="card">
        <h2 className="text-xl font-semibold mb-6">Upload Files</h2>

        <div className="space-y-6">
          <div>
            <label className="flex items-center cursor-pointer">
              <input
                type="checkbox"
                checked={noCompress}
                onChange={(e) => setNoCompress(e.target.checked)}
                className="mr-2"
              />
              <span className="text-sm font-medium text-gray-300">
                Disable Compression (for images and videos)
              </span>
            </label>
          </div>

          {/* Upload Progress */}
          {isUploading && uploadProgress && (
            <ProgressBar 
              progress={uploadProgress} 
            />
          )}

          <div className="flex gap-4">
            <Button
              onClick={handleFileSelection}
              className="btn-primary flex-1"
              disabled={isUploading}
            >
              {isUploading ? "Uploading..." : "Select Files to Upload"}
            </Button>
          </div>
        </div>
      </div>

      {self && (
        <div className="card max-w-2xl mx-auto">
          <h3 className="text-lg font-semibold mb-4">Storage Usage</h3>
          <div className="space-y-4">
            {/* File Count */}
            <div className="flex justify-between text-sm">
              <span>Files:</span>
              <span className="font-medium">
                {self.file_count.toLocaleString()}
              </span>
            </div>

            {/* Total Usage */}
            <div className="flex justify-between text-sm">
              <span>Total Size:</span>
              <span className="font-medium">
                {FormatBytes(self.total_size)}
              </span>
            </div>

            {/* Only show quota information if available */}
            {self.total_available_quota && self.total_available_quota > 0 && (
              <>
                {/* Progress Bar */}
                <div className="space-y-2">
                  <div className="flex justify-between text-sm">
                    <span>Quota Used:</span>
                    <span className="font-medium">
                      {FormatBytes(self.total_size)} of{" "}
                      {FormatBytes(self.total_available_quota)}
                    </span>
                  </div>
                  <div className="w-full bg-gray-700 rounded-full h-2.5">
                    <div
                      className={`h-2.5 rounded-full transition-all duration-300 ${
                        self.total_size / self.total_available_quota > 0.8
                          ? "bg-red-500"
                          : self.total_size / self.total_available_quota > 0.6
                            ? "bg-yellow-500"
                            : "bg-green-500"
                      }`}
                      style={{
                        width: `${Math.min(100, (self.total_size / self.total_available_quota) * 100)}%`,
                      }}
                    ></div>
                  </div>
                  <div className="flex justify-between text-xs text-gray-400">
                    <span>
                      {(
                        (self.total_size / self.total_available_quota) *
                        100
                      ).toFixed(1)}
                      % used
                    </span>
                    <span
                      className={`${
                        self.total_size / self.total_available_quota > 0.8
                          ? "text-red-400"
                          : self.total_size / self.total_available_quota > 0.6
                            ? "text-yellow-400"
                            : "text-green-400"
                      }`}
                    >
                      {FormatBytes(
                        Math.max(
                          0,
                          self.total_available_quota - self.total_size,
                        ),
                      )}{" "}
                      remaining
                    </span>
                  </div>
                </div>

                {/* Quota Breakdown - excluding free quota */}
                <div className="space-y-2 pt-2 border-t border-gray-700">
                  {(self.quota ?? 0) > 0 && (
                    <div className="flex justify-between text-sm">
                      <span>Paid Quota:</span>
                      <span className="font-medium">
                        {FormatBytes(self.quota!)}
                      </span>
                    </div>
                  )}
                  {(self.paid_until ?? 0) > 0 && (
                    <div className="flex justify-between text-sm">
                      <span>Expires:</span>
                      <div className="text-right">
                        <div className="font-medium">
                          {new Date(
                            self.paid_until! * 1000,
                          ).toLocaleDateString()}
                        </div>
                        <div className="text-xs text-gray-400">
                          {(() => {
                            const now = Date.now() / 1000;
                            const daysLeft = Math.max(
                              0,
                              Math.ceil(
                                (self.paid_until! - now) / (24 * 60 * 60),
                              ),
                            );
                            return daysLeft > 0
                              ? `${daysLeft} days left`
                              : "Expired";
                          })()}
                        </div>
                      </div>
                    </div>
                  )}
                </div>
              </>
            )}
          </div>
          <Button
            onClick={() => setShowPaymentFlow(!showPaymentFlow)}
            className="btn-primary w-full mt-4"
          >
            {showPaymentFlow ? "Hide" : "Show"} Payment Options
          </Button>
        </div>
      )}

      {showPaymentFlow && pub && (
        <div className="card">
          <PaymentFlow
            route96={new Route96(url, pub)}
            onPaymentRequested={(pr) => {
              console.log("Payment requested:", pr);
            }}
            userInfo={self}
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
          <div className="space-y-4">
            {results.map((result, index) => (
              <div
                key={index}
                className="bg-gray-800 border border-gray-700 rounded-lg p-4"
              >
                <div className="flex items-start justify-between mb-3">
                  <div className="flex-1">
                    <h4 className="font-medium text-green-400 mb-1">
                      ✅ Upload Successful
                    </h4>
                    <p className="text-sm text-gray-400">
                      {new Date(
                        (result.uploaded || Date.now() / 1000) * 1000,
                      ).toLocaleString()}
                    </p>
                  </div>
                  <div className="text-right">
                    <span className="text-xs bg-blue-900/50 text-blue-300 px-2 py-1 rounded">
                      {result.type || "Unknown type"}
                    </span>
                  </div>
                </div>

                <div className="grid grid-cols-1 md:grid-cols-2 gap-4 mb-4">
                  <div>
                    <p className="text-sm text-gray-400">File Size</p>
                    <p className="font-medium">
                      {FormatBytes(result.size || 0)}
                    </p>
                  </div>
                </div>

                <div className="space-y-2">
                  {result.url && (
                    <div>
                      <p className="text-sm text-gray-400 mb-1">File URL</p>
                      <div className="flex items-center gap-2">
                        <code className="text-xs bg-gray-900 text-green-400 px-2 py-1 rounded flex-1 overflow-hidden">
                          {result.url}
                        </code>
                        <button
                          onClick={() =>
                            navigator.clipboard.writeText(result.url!)
                          }
                          className="text-xs bg-blue-600 hover:bg-blue-700 text-white px-2 py-1 rounded transition-colors"
                          title="Copy URL"
                        >
                          Copy
                        </button>
                      </div>
                    </div>
                  )}

                  <div>
                    <p className="text-sm text-gray-400 mb-1">
                      File Hash (SHA256)
                    </p>
                    <code className="text-xs bg-gray-900 text-gray-400 px-2 py-1 rounded block overflow-hidden">
                      {result.sha256}
                    </code>
                  </div>
                </div>

                <details className="mt-4">
                  <summary className="text-sm text-gray-400 cursor-pointer hover:text-gray-300">
                    Show raw JSON data
                  </summary>
                  <pre className="text-xs bg-gray-900 text-gray-300 p-3 rounded mt-2 overflow-auto">
                    {JSON.stringify(result, undefined, 2)}
                  </pre>
                </details>
              </div>
            ))}
          </div>
        </div>
      )}
    </div>
  );
}
