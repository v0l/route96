import { useState } from "react";
import Button from "../components/button";
import FileList from "./files";
import { openFile } from "../upload";
import { Blossom } from "../upload/blossom";
import useLogin from "../hooks/login";
import usePublisher from "../hooks/publisher";
import { Nip96 } from "../upload/nip96";

export default function Upload() {
    const [type, setType] = useState<"blossom" | "nip96">("nip96");
    const [noCompress, setNoCompress] = useState(false);
    const [toUpload, setToUpload] = useState<File>();
    const [error, setError] = useState<string>();
    const [results, setResults] = useState<Array<object>>([]);
    const login = useLogin();
    const pub = usePublisher();

    async function doUpload() {
        if (!pub) return;
        if (!toUpload) return;
        try {
            setError(undefined);
            const url = `${location.protocol}//${location.host}`;
            if (type === "blossom") {
                const uploader = new Blossom(url, pub);
                const result = await uploader.upload(toUpload);
                setResults(s => [...s, result]);
            }
            if (type === "nip96") {
                const uploader = new Nip96(url, pub);
                await uploader.loadInfo();
                const result = await uploader.upload(toUpload);
                setResults(s => [...s, result]);
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

    async function listUploads() {
        if (!pub) return;
        try {
            setError(undefined);
            const url = `${location.protocol}//${location.host}`;
            const uploader = new Nip96(url, pub);
            await uploader.loadInfo();
            const result = await uploader.listFiles();
            setResults(s => [...s, result]);
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
            <Button disabled={login === undefined} onClick={listUploads}>List Uploads</Button>
            {error && <b className="text-red-500">{error}</b>}
            <pre className="text-xs font-monospace overflow-wrap">{JSON.stringify(results, undefined, 2)}</pre>
        </div>
    );
}
