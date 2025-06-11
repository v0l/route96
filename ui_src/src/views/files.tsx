import { NostrEvent, NostrLink } from "@snort/system";
import { useState } from "react";
import { FormatBytes } from "../const";
import classNames from "classnames";
import Profile from "../components/profile";

interface FileInfo {
  id: string;
  url: string;
  name?: string;
  type?: string;
  size?: number;
  uploader?: Array<string>;
}

export default function FileList({
  files,
  pages,
  page,
  onPage,
  onDelete,
}: {
  files: Array<File | NostrEvent | FileInfo>;
  pages?: number;
  page?: number;
  onPage?: (n: number) => void;
  onDelete?: (id: string) => void;
}) {
  const [viewType, setViewType] = useState<"grid" | "list">("grid");
  if (files.length === 0) {
    return <b className="text-gray-400">No Files</b>;
  }

  function renderInner(f: FileInfo) {
    if (
      f.type?.startsWith("image/") ||
      f.type?.startsWith("video/") ||
      !f.type
    ) {
      return (
        <img
          src={f.url.replace(`/${f.id}`, `/thumb/${f.id}`)}
          className="w-full h-full object-contain object-center"
          loading="lazy"
        />
      );
    }
  }

  function getInfo(f: File | NostrEvent | FileInfo): FileInfo {
    if ("url" in f) {
      return f;
    }
    if ("created_at" in f) {
      return {
        id: f.tags.find((a) => a[0] === "x")![1],
        url: f.tags.find((a) => a[0] === "url")![1],
        name: f.content,
        type: f.tags.find((a) => a[0] === "m")?.at(1),
        size: Number(f.tags.find((a) => a[0] === "size")?.at(1)),
        uploader: "uploader" in f ? (f.uploader as Array<string>) : undefined,
      };
    } else {
      return {
        id: f.name,
        url: URL.createObjectURL(f),
        name: f.name,
        type: f.type,
        size: f.size,
      };
    }
  }

  function pageButtons(page: number, n: number) {
    const ret = [];
    const start = 0;

    for (let x = start; x < n; x++) {
      ret.push(
        <button
          key={x}
          onClick={() => onPage?.(x)}
          className={classNames(
            "px-3 py-2 text-sm font-medium border transition-colors",
            {
              "rounded-l-md": x === start,
              "rounded-r-md": x + 1 === n,
              "bg-blue-600 text-white border-blue-600": page === x,
              "bg-white text-gray-700 border-gray-300 hover:bg-gray-50": page !== x,
            },
          )}
        >
          {x + 1}
        </button>,
      );
    }

    return ret;
  }

  function showGrid() {
    return (
      <div className="grid gap-4 grid-cols-2 sm:grid-cols-3 md:grid-cols-4 lg:grid-cols-6 xl:grid-cols-8">
        {files.map((a) => {
          const info = getInfo(a);

          return (
            <div
              key={info.id}
              className="group relative rounded-lg aspect-square overflow-hidden bg-gray-100 border border-gray-200 hover:shadow-md transition-shadow"
            >
              <div className="absolute inset-0 flex flex-col items-center justify-center p-2 text-xs text-center opacity-0 group-hover:opacity-100 bg-black/75 text-white transition-opacity">
                <div className="font-medium mb-1">
                  {(info.name?.length ?? 0) === 0
                    ? "Untitled"
                    : info.name!.length > 20
                      ? `${info.name?.substring(0, 10)}...${info.name?.substring(info.name.length - 10)}`
                      : info.name}
                </div>
                <div className="text-gray-300 mb-1">
                  {info.size && !isNaN(info.size)
                    ? FormatBytes(info.size, 2)
                    : ""}
                </div>
                <div className="text-gray-300 mb-2">{info.type}</div>
                <div className="flex gap-2">
                  <a href={info.url} className="bg-blue-600 hover:bg-blue-700 px-2 py-1 rounded text-xs" target="_blank">
                    View
                  </a>
                  {onDelete && (
                    <button
                      onClick={(e) => {
                        e.preventDefault();
                        onDelete?.(info.id);
                      }}
                      className="bg-red-600 hover:bg-red-700 px-2 py-1 rounded text-xs"
                    >
                      Delete
                    </button>
                  )}
                </div>
                {info.uploader &&
                  info.uploader.map((a, idx) => (
                    <Profile key={idx} link={NostrLink.publicKey(a)} size={20} />
                  ))}
              </div>
              {renderInner(info)}
            </div>
          );
        })}
      </div>
    );
  }

  function showList() {
    return (
      <div className="overflow-x-auto">
        <table className="min-w-full bg-white border border-gray-200 rounded-lg">
          <thead className="bg-gray-50">
            <tr>
              <th className="px-4 py-3 text-left text-xs font-medium text-gray-500 uppercase tracking-wider border-b border-gray-200">
                Preview
              </th>
              <th className="px-4 py-3 text-left text-xs font-medium text-gray-500 uppercase tracking-wider border-b border-gray-200">
                Name
              </th>
              <th className="px-4 py-3 text-left text-xs font-medium text-gray-500 uppercase tracking-wider border-b border-gray-200">
                Type
              </th>
              <th className="px-4 py-3 text-left text-xs font-medium text-gray-500 uppercase tracking-wider border-b border-gray-200">
                Size
              </th>
              {files.some((i) => "uploader" in i) && (
                <th className="px-4 py-3 text-left text-xs font-medium text-gray-500 uppercase tracking-wider border-b border-gray-200">
                  Uploader
                </th>
              )}
              <th className="px-4 py-3 text-left text-xs font-medium text-gray-500 uppercase tracking-wider border-b border-gray-200">
                Actions
              </th>
            </tr>
          </thead>
          <tbody className="divide-y divide-gray-200">
            {files.map((a) => {
              const info = getInfo(a);
              return (
                <tr key={info.id} className="hover:bg-gray-50">
                  <td className="px-4 py-3 w-16">
                    <div className="w-12 h-12 bg-gray-100 rounded overflow-hidden">
                      {renderInner(info)}
                    </div>
                  </td>
                  <td className="px-4 py-3 text-sm text-gray-900 break-all max-w-xs">
                    {(info.name?.length ?? 0) === 0 ? "<Untitled>" : info.name}
                  </td>
                  <td className="px-4 py-3 text-sm text-gray-500">
                    {info.type}
                  </td>
                  <td className="px-4 py-3 text-sm text-gray-500">
                    {info.size && !isNaN(info.size)
                      ? FormatBytes(info.size, 2)
                      : ""}
                  </td>
                  {info.uploader && (
                    <td className="px-4 py-3">
                      {info.uploader.map((a, idx) => (
                        <Profile key={idx} link={NostrLink.publicKey(a)} size={20} />
                      ))}
                    </td>
                  )}
                  <td className="px-4 py-3">
                    <div className="flex gap-2">
                      <a href={info.url} className="bg-blue-600 hover:bg-blue-700 text-white px-3 py-1 rounded text-xs" target="_blank">
                        View
                      </a>
                      {onDelete && (
                        <button
                          onClick={(e) => {
                            e.preventDefault();
                            onDelete?.(info.id);
                          }}
                          className="bg-red-600 hover:bg-red-700 text-white px-3 py-1 rounded text-xs"
                        >
                          Delete
                        </button>
                      )}
                    </div>
                  </td>
                </tr>
              );
            })}
          </tbody>
        </table>
      </div>
    );
  }

  return (
    <div className="space-y-4">
      <div className="flex justify-between items-center">
        <div className="flex rounded-lg border border-gray-300 overflow-hidden">
          <button
            onClick={() => setViewType("grid")}
            className={`px-4 py-2 text-sm font-medium transition-colors ${
              viewType === "grid" 
                ? "bg-blue-600 text-white" 
                : "bg-white text-gray-700 hover:bg-gray-50"
            }`}
          >
            Grid
          </button>
          <button
            onClick={() => setViewType("list")}
            className={`px-4 py-2 text-sm font-medium transition-colors border-l border-gray-300 ${
              viewType === "list" 
                ? "bg-blue-600 text-white" 
                : "bg-white text-gray-700 hover:bg-gray-50"
            }`}
          >
            List
          </button>
        </div>
      </div>
      
      {viewType === "grid" ? showGrid() : showList()}
      
      {pages !== undefined && pages > 1 && (
        <div className="flex justify-center">
          <div className="flex rounded-lg border border-gray-300 overflow-hidden">
            {pageButtons(page ?? 0, pages)}
          </div>
        </div>
      )}
    </div>
  );
}
