import { useEffect, useState } from "react";
import Button from "../components/button";
import FileList from "./files";
import { openFile } from "../upload";
import { Blossom } from "../upload/blossom";
import useLogin from "../hooks/login";
import usePublisher from "../hooks/publisher";
import { Nip96, Nip96FileList } from "../upload/nip96";

export default function Upload() {
  const [type, setType] = useState<"blossom" | "nip96">("nip96");
  const [noCompress, setNoCompress] = useState(false);
  const [toUpload, setToUpload] = useState<File>();
  const [error, setError] = useState<string>();
  const [results, setResults] = useState<Array<object>>([]);
  const [listedFiles, setListedFiles] = useState<Nip96FileList>();
  const [listedPage, setListedPage] = useState(0);

  const login = useLogin();
  const pub = usePublisher();

  const url = `${location.protocol}//${location.host}`;
  //const url = "https://files.v0l.io";
  async function doUpload() {
    if (!pub) return;
    if (!toUpload) return;
    try {
      setError(undefined);
      if (type === "blossom") {
        const uploader = new Blossom(url, pub);
        const result = await uploader.upload(toUpload);
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
      const result = await uploader.listFiles(n, 12);
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

  useEffect(() => {
    listUploads(listedPage);
  }, [listedPage]);

  return (
    <div className="flex flex-col gap-2 bg-neutral-700 p-8 rounded-xl w-full">
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

      {type === "nip96" && (
        <div
          className="flex gap-2 cursor-pointer"
          onClick={() => setNoCompress((s) => !s)}
        >
          Disable Compression
          <input type="checkbox" checked={noCompress} />
        </div>
      )}

      <Button
        onClick={async () => {
          const f = await openFile();
          setToUpload(f);
        }}
      >
        Choose Files
      </Button>
      <FileList files={toUpload ? [toUpload] : []} />
      <Button onClick={doUpload} disabled={login === undefined}>
        Upload
      </Button>
      <Button disabled={login === undefined} onClick={() => listUploads(0)}>
        List Uploads
      </Button>
      {listedFiles && (
        <FileList
          files={listedFiles.files}
          pages={listedFiles.total / listedFiles.count}
          page={listedFiles.page}
          onPage={(x) => setListedPage(x)}
        />
      )}
      {error && <b className="text-red-500">{error}</b>}
      <pre className="text-xs font-monospace overflow-wrap">
        {JSON.stringify(results, undefined, 2)}
      </pre>
    </div>
  );
}
