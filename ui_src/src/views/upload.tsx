import { useEffect, useState, useCallback } from "react";
import Button from "../components/button";
import FileList from "./files";
import PaymentFlow from "../components/payment";
import ProgressBar from "../components/progress-bar";
import MirrorSuggestions from "../components/mirror-suggestions";
import { useBlossomServers } from "../hooks/use-blossom-servers";
import { openFiles } from "../upload";
import { Blossom, BlobDescriptor } from "../upload/blossom";
import useLogin from "../hooks/login";
import usePublisher from "../hooks/publisher";
import { Nip96, Nip96FileList } from "../upload/nip96";
import { AdminSelf, Route96 } from "../upload/admin";
import { FormatBytes, ServerUrl } from "../const";
import { UploadProgress } from "../upload/progress";

export default function Upload() {
  const [stripMetadata, setStripMetadata] = useState(true);
  const [self, setSelf] = useState<AdminSelf>();
  const [error, setError] = useState<string>();
  const [results, setResults] = useState<Array<BlobDescriptor>>([]);
  const [listedFiles, setListedFiles] = useState<Nip96FileList>();
  const [listedPage, setListedPage] = useState(0);
  const [showPaymentFlow, setShowPaymentFlow] = useState(false);
  const [isUploading, setIsUploading] = useState(false);
  const [uploadProgress, setUploadProgress] = useState<UploadProgress>();

  const blossomServers = useBlossomServers();

  const login = useLogin();
  const pub = usePublisher();

  const shouldCompress = (file: File) => {
    return file.type.startsWith('video/') || file.type.startsWith('image/');
  };

  async function doUpload(file: File) {
    if (!pub) return;
    if (!file) return;
    if (isUploading) return;

    try {
      setError(undefined);
      setIsUploading(true);
      setUploadProgress(undefined);

      const onProgress = (progress: UploadProgress) => {
        setUploadProgress(progress);
      };

      const uploader = new Blossom(ServerUrl, pub);
      const useCompression = shouldCompress(file) && stripMetadata;
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
        const uploader = new Nip96(ServerUrl, pub);
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
    [pub],
  );

  async function deleteFile(id: string) {
    if (!pub) return;
    try {
      setError(undefined);
      const uploader = new Blossom(ServerUrl, pub);
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
      const r96 = new Route96(ServerUrl, pub);
      r96.getSelf().then((v) => setSelf(v.data));
    }
  }, [pub, self]);

  if (!login) {
    return (
      <div className="max-w-md mx-auto bg-neutral-900 border border-neutral-800 rounded-sm p-4">
        <h2 className="text-lg font-medium mb-2 text-white">
          Welcome to {window.location.hostname}
        </h2>
        <p className="text-neutral-400 text-sm">
          Please log in to start uploading files.
        </p>
      </div>
    );
  }

  return (
    <div className="space-y-4">
      {error && (
        <div className="bg-red-950 border border-red-900 text-red-200 px-3 py-2 rounded-sm text-sm">
          {error}
        </div>
      )}

      <div className="flex flex-wrap gap-4">
        {/* Upload Widget */}
        <div className="flex-1 min-w-72 bg-neutral-900 border border-neutral-800 rounded-sm">
          <div className="p-3">
            <h3 className="text-sm font-medium mb-3 text-white">Upload Files</h3>
            <div className="space-y-3">
              <label className="flex items-center cursor-pointer">
                <input
                  type="checkbox"
                  checked={stripMetadata}
                  onChange={(e) => setStripMetadata(e.target.checked)}
                  className="mr-2 w-3.5 h-3.5 rounded-sm bg-neutral-800 border-neutral-700"
                />
                <span className="text-xs text-neutral-400">
                  Strip metadata (images)
                </span>
              </label>

              {isUploading && uploadProgress && (
                <ProgressBar progress={uploadProgress} />
              )}

              <Button
                onClick={handleFileSelection}
                className="w-full"
                disabled={isUploading}
              >
                {isUploading ? "Uploading..." : "Select Files"}
              </Button>
            </div>
          </div>
        </div>

        {/* Storage Usage Widget */}
        {self && (
          <div className="flex-1 min-w-72 bg-neutral-900 border border-neutral-800 rounded-sm">
            <div className="p-3">
              <h3 className="text-sm font-medium mb-3 text-white">Storage</h3>
              <div className="space-y-2 text-xs">
                <div className="flex justify-between">
                  <span className="text-neutral-500">Files:</span>
                  <span className="text-white">{self.file_count.toLocaleString()}</span>
                </div>

                <div className="flex justify-between">
                  <span className="text-neutral-500">Size:</span>
                  <span className="text-white">{FormatBytes(self.total_size)}</span>
                </div>

                {self.total_available_quota && self.total_available_quota > 0 && (
                  <>
                    <div className="flex justify-between">
                      <span className="text-neutral-500">Quota:</span>
                      <span className="text-white">
                        {FormatBytes(self.total_size)} / {FormatBytes(self.total_available_quota)}
                      </span>
                    </div>
                    <div className="w-full bg-neutral-800 rounded-sm h-1.5">
                      <div 
                        className="bg-white h-1.5 rounded-sm transition-all"
                        style={{ width: `${Math.min(100, (self.total_size / self.total_available_quota) * 100)}%` }}
                      />
                    </div>
                    <div className="flex justify-between text-neutral-500">
                      <span>
                        {((self.total_size / self.total_available_quota) * 100).toFixed(1)}%
                      </span>
                      <span className={
                        self.total_size / self.total_available_quota > 0.8
                          ? "text-red-400"
                          : self.total_size / self.total_available_quota > 0.6
                            ? "text-yellow-400"
                            : "text-green-400"
                      }>
                        {FormatBytes(Math.max(0, self.total_available_quota - self.total_size))} free
                      </span>
                    </div>

                    {(self.quota ?? 0) > 0 && (
                      <div className="flex justify-between pt-2 border-t border-neutral-800">
                        <span className="text-neutral-500">Paid:</span>
                        <span className="text-white">{FormatBytes(self.quota!)}</span>
                      </div>
                    )}
                    {(self.paid_until ?? 0) > 0 && (
                      <div className="flex justify-between">
                        <span className="text-neutral-500">Expires:</span>
                        <span className="text-white">
                          {new Date(self.paid_until! * 1000).toLocaleDateString()}
                        </span>
                      </div>
                    )}
                  </>
                )}
                <Button
                  onClick={() => setShowPaymentFlow(!showPaymentFlow)}
                  className="w-full mt-2"
                  variant="secondary"
                  size="sm"
                >
                  {showPaymentFlow ? "Hide" : "Payment Options"}
                </Button>
              </div>
            </div>
          </div>
        )}

        {/* Payment Flow Widget */}
        {showPaymentFlow && pub && (
          <div className="flex-1 min-w-72">
            <PaymentFlow
              route96={new Route96(ServerUrl, pub)}
              onPaymentRequested={(pr) => {
                console.log("Payment requested:", pr);
              }}
              userInfo={self}
            />
          </div>
        )}

        {/* Mirror Suggestions Widget */}
        {blossomServers && blossomServers.length > 1 && (
          <div className="w-full">
            <MirrorSuggestions servers={blossomServers} />
          </div>
        )}

        {/* Upload Results Widget */}
        {results.length > 0 && (
          <div className="w-full bg-neutral-900 border border-neutral-800 rounded-sm">
            <div className="p-3">
              <h3 className="text-sm font-medium mb-3 text-white">Results</h3>
              <div className="space-y-2">
                {results.map((result, index) => (
                  <div
                    key={index}
                    className="bg-neutral-950 border border-neutral-800 rounded-sm p-2"
                  >
                    <div className="flex items-start justify-between mb-2">
                      <div>
                        <span className="text-xs text-green-400">Uploaded</span>
                        <span className="text-xs text-neutral-500 ml-2">
                          {new Date((result.uploaded || Date.now() / 1000) * 1000).toLocaleString()}
                        </span>
                      </div>
                      <span className="text-xs bg-neutral-800 text-neutral-300 px-1.5 py-0.5 rounded-sm">
                        {result.type || "Unknown"}
                      </span>
                    </div>

                    <div className="text-xs text-neutral-400 mb-2">
                      {FormatBytes(result.size || 0)}
                    </div>

                    {result.url && (
                      <div className="mb-2">
                        <div className="flex items-center gap-1">
                          <code className="text-xs bg-neutral-950 text-green-400 px-1.5 py-0.5 rounded-sm flex-1 overflow-hidden truncate">
                            {result.url}
                          </code>
                          <button
                            onClick={() => navigator.clipboard.writeText(result.url!)}
                            className="text-xs bg-neutral-800 hover:bg-neutral-700 text-white px-1.5 py-0.5 rounded-sm"
                          >
                            Copy
                          </button>
                        </div>
                      </div>
                    )}

                    <div>
                      <span className="text-xs text-neutral-500">SHA256: </span>
                      <code className="text-xs text-neutral-600 break-all">
                        {result.sha256}
                      </code>
                    </div>

                    <details className="mt-2">
                      <summary className="text-xs text-neutral-600 cursor-pointer hover:text-neutral-400">
                        Raw JSON
                      </summary>
                      <pre className="text-xs bg-neutral-950 text-neutral-400 p-2 rounded-sm mt-1 overflow-auto">
                        {JSON.stringify(result, undefined, 2)}
                      </pre>
                    </details>
                  </div>
                ))}
              </div>
            </div>
          </div>
        )}

        {/* Files Widget */}
        <div className="w-full bg-neutral-900 border border-neutral-800 rounded-sm">
          <div className="p-3">
            <div className="flex justify-between items-center mb-3">
              <h3 className="text-sm font-medium text-white">Your Files</h3>
              {!listedFiles && (
                <Button onClick={() => listUploads(0)} size="sm">
                  Load
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
        </div>
      </div>
    </div>
  );
}
