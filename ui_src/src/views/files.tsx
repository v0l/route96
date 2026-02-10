import { NostrEvent, NostrLink } from "@snort/system";
import { useState, useEffect } from "react";
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
  adminMode,
}: {
  files: Array<File | NostrEvent | FileInfo>;
  pages?: number;
  page?: number;
  onPage?: (n: number) => void;
  onDelete?: (id: string) => void;
  adminMode?: boolean;
}) {
  const [viewType, setViewType] = useState<"grid" | "list">("grid");
  const [gridCols, setGridCols] = useState(() => {
    const saved = localStorage.getItem("file-grid-columns");
    return saved ? parseInt(saved) : 6;
  });

  useEffect(() => {
    localStorage.setItem("file-grid-columns", gridCols.toString());
  }, [gridCols]);
  
  if (files.length === 0) {
    return <span className="text-neutral-500 text-sm">No Files</span>;
  }

  function getGridClass() {
    const baseClasses = "grid gap-2";
    switch (gridCols) {
      case 2:
        return `${baseClasses} grid-cols-1 sm:grid-cols-2`;
      case 3:
        return `${baseClasses} grid-cols-2 sm:grid-cols-3`;
      case 4:
        return `${baseClasses} grid-cols-2 sm:grid-cols-3 md:grid-cols-4`;
      case 5:
        return `${baseClasses} grid-cols-2 sm:grid-cols-3 md:grid-cols-4 lg:grid-cols-5`;
      case 6:
        return `${baseClasses} grid-cols-2 sm:grid-cols-3 md:grid-cols-4 lg:grid-cols-5 xl:grid-cols-6`;
      case 8:
        return `${baseClasses} grid-cols-2 sm:grid-cols-3 md:grid-cols-4 lg:grid-cols-6 xl:grid-cols-8`;
      case 10:
        return `${baseClasses} grid-cols-2 sm:grid-cols-4 md:grid-cols-6 lg:grid-cols-8 xl:grid-cols-10`;
      case 12:
        return `${baseClasses} grid-cols-3 sm:grid-cols-4 md:grid-cols-6 lg:grid-cols-9 xl:grid-cols-12`;
      default:
        return `${baseClasses} grid-cols-2 sm:grid-cols-3 md:grid-cols-4 lg:grid-cols-5 xl:grid-cols-6`;
    }
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

  function pageButtons(page: number, totalPages: number) {
    const ret = [];
    const maxVisiblePages = 5;
    
    ret.push(
      <button
        key="prev"
        onClick={() => onPage?.(page - 1)}
        disabled={page === 0}
        className="px-2 py-1 text-xs border border-neutral-800 bg-neutral-900 text-neutral-300 hover:bg-neutral-800 disabled:opacity-50 disabled:cursor-not-allowed rounded-l-sm transition-colors"
      >
        &larr;
      </button>
    );

    let startPage = Math.max(0, page - Math.floor(maxVisiblePages / 2));
    let endPage = Math.min(totalPages - 1, startPage + maxVisiblePages - 1);
    
    if (endPage - startPage < maxVisiblePages - 1) {
      startPage = Math.max(0, endPage - maxVisiblePages + 1);
    }

    if (startPage > 0) {
      ret.push(
        <button
          key={0}
          onClick={() => onPage?.(0)}
          className="px-2 py-1 text-xs border-t border-b border-neutral-800 bg-neutral-900 text-neutral-300 hover:bg-neutral-800 transition-colors"
        >
          1
        </button>
      );
      if (startPage > 1) {
        ret.push(
          <span key="ellipsis1" className="px-2 py-1 text-xs text-neutral-600 border-t border-b border-neutral-800 bg-neutral-900">
            ...
          </span>
        );
      }
    }

    for (let x = startPage; x <= endPage; x++) {
      ret.push(
        <button
          key={x}
          onClick={() => onPage?.(x)}
          className={classNames(
            "px-2 py-1 text-xs border-t border-b border-neutral-800 transition-colors",
            {
              "bg-white text-black": page === x,
              "bg-neutral-900 text-neutral-300 hover:bg-neutral-800": page !== x,
            },
          )}
        >
          {x + 1}
        </button>,
      );
    }

    if (endPage < totalPages - 1) {
      if (endPage < totalPages - 2) {
        ret.push(
          <span key="ellipsis2" className="px-2 py-1 text-xs text-neutral-600 border-t border-b border-neutral-800 bg-neutral-900">
            ...
          </span>
        );
      }
      ret.push(
        <button
          key={totalPages - 1}
          onClick={() => onPage?.(totalPages - 1)}
          className="px-2 py-1 text-xs border-t border-b border-neutral-800 bg-neutral-900 text-neutral-300 hover:bg-neutral-800 transition-colors"
        >
          {totalPages}
        </button>
      );
    }

    ret.push(
      <button
        key="next"
        onClick={() => onPage?.(page + 1)}
        disabled={page === totalPages - 1}
        className="px-2 py-1 text-xs border border-neutral-800 bg-neutral-900 text-neutral-300 hover:bg-neutral-800 disabled:opacity-50 disabled:cursor-not-allowed rounded-r-sm transition-colors"
      >
        &rarr;
      </button>
    );

    return ret;
  }

  function showGrid() {
    return (
      <div className={getGridClass()}>
        {files.map((a) => {
          const info = getInfo(a);

          return (
            <div
              key={info.id}
              className="group relative rounded-sm aspect-square overflow-hidden bg-neutral-900 border border-neutral-800 hover:border-neutral-700 transition-colors"
            >
              <div className="absolute inset-0 flex flex-col items-center justify-center p-2 text-xs text-center opacity-0 group-hover:opacity-100 bg-black/80 text-white transition-opacity">
                <div className="font-medium mb-1">
                  {(info.name?.length ?? 0) === 0
                    ? "Untitled"
                    : info.name!.length > 20
                      ? `${info.name?.substring(0, 10)}...${info.name?.substring(info.name.length - 10)}`
                      : info.name}
                </div>
                <div className="text-neutral-400 mb-1">
                  {info.size && !isNaN(info.size)
                    ? FormatBytes(info.size, 2)
                    : ""}
                </div>
                <div className="text-neutral-500 mb-2">{info.type}</div>
                <div className="flex gap-1">
                  <a
                    href={info.url}
                    className="bg-neutral-800 hover:bg-neutral-700 text-white px-2 py-1 rounded-sm text-xs"
                    target="_blank"
                  >
                    View
                  </a>
                  {onDelete && (
                    <button
                      onClick={(e) => {
                        e.preventDefault();
                        onDelete?.(info.id);
                      }}
                      className="bg-red-600 hover:bg-red-500 text-white px-2 py-1 rounded-sm text-xs"
                    >
                      Delete
                    </button>
                  )}
                </div>
                {info.uploader &&
                  info.uploader.map((a, idx) => (
                    <Profile
                      key={idx}
                      link={NostrLink.publicKey(a)}
                      size={20}
                      adminMode={adminMode}
                    />
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
        <table className="min-w-full bg-neutral-900 border border-neutral-800 rounded-sm text-xs">
          <thead className="bg-neutral-950">
            <tr>
              <th className="px-2 py-1.5 text-left text-xs font-medium text-neutral-500 uppercase border-b border-neutral-800">
                Preview
              </th>
              <th className="px-2 py-1.5 text-left text-xs font-medium text-neutral-500 uppercase border-b border-neutral-800">
                Name
              </th>
              <th className="px-2 py-1.5 text-left text-xs font-medium text-neutral-500 uppercase border-b border-neutral-800">
                Type
              </th>
              <th className="px-2 py-1.5 text-left text-xs font-medium text-neutral-500 uppercase border-b border-neutral-800">
                Size
              </th>
              {files.some((i) => "uploader" in i) && (
                <th className="px-2 py-1.5 text-left text-xs font-medium text-neutral-500 uppercase border-b border-neutral-800">
                  Uploader
                </th>
              )}
              <th className="px-2 py-1.5 text-left text-xs font-medium text-neutral-500 uppercase border-b border-neutral-800">
                Actions
              </th>
            </tr>
          </thead>
          <tbody className="divide-y divide-neutral-800">
            {files.map((a) => {
              const info = getInfo(a);
              return (
                <tr key={info.id} className="hover:bg-neutral-800/50">
                  <td className="px-2 py-1.5 w-12">
                    <div className="w-10 h-10 bg-neutral-800 rounded-sm overflow-hidden">
                      {renderInner(info)}
                    </div>
                  </td>
                  <td className="px-2 py-1.5 text-neutral-200 break-all max-w-xs">
                    {(info.name?.length ?? 0) === 0 ? "<Untitled>" : info.name}
                  </td>
                  <td className="px-2 py-1.5 text-neutral-500">
                    {info.type}
                  </td>
                  <td className="px-2 py-1.5 text-neutral-500">
                    {info.size && !isNaN(info.size)
                      ? FormatBytes(info.size, 2)
                      : ""}
                  </td>
                  {info.uploader && (
                    <td className="px-2 py-1.5">
                      {info.uploader.map((a, idx) => (
                        <Profile
                          key={idx}
                          link={NostrLink.publicKey(a)}
                          size={20}
                          adminMode={adminMode}
                        />
                      ))}
                    </td>
                  )}
                  <td className="px-2 py-1.5">
                    <div className="flex gap-1">
                      <a
                        href={info.url}
                        className="bg-neutral-800 hover:bg-neutral-700 text-white px-2 py-1 rounded-sm text-xs"
                        target="_blank"
                      >
                        View
                      </a>
                      {onDelete && (
                        <button
                          onClick={(e) => {
                            e.preventDefault();
                            onDelete?.(info.id);
                          }}
                          className="bg-red-600 hover:bg-red-500 text-white px-2 py-1 rounded-sm text-xs"
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
    <div className="space-y-3">
      <div className="flex justify-between items-center flex-wrap gap-2">
        <div className="flex rounded-sm border border-neutral-800 overflow-hidden">
          <button
            onClick={() => setViewType("grid")}
            className={`px-2 py-1 text-xs transition-colors ${
              viewType === "grid"
                ? "bg-white text-black"
                : "bg-neutral-900 text-neutral-400 hover:bg-neutral-800"
            }`}
          >
            Grid
          </button>
          <button
            onClick={() => setViewType("list")}
            className={`px-2 py-1 text-xs transition-colors border-l border-neutral-800 ${
              viewType === "list"
                ? "bg-white text-black"
                : "bg-neutral-900 text-neutral-400 hover:bg-neutral-800"
            }`}
          >
            List
          </button>
        </div>
        
        {viewType === "grid" && (
          <div className="flex items-center gap-2">
            <label className="text-xs text-neutral-500">Cols:</label>
            <select
              value={gridCols}
              onChange={(e) => setGridCols(parseInt(e.target.value))}
              className="h-6 w-14 rounded-sm border border-neutral-800 bg-neutral-900 text-neutral-300 px-1 text-xs"
            >
              <option value={2}>2</option>
              <option value={3}>3</option>
              <option value={4}>4</option>
              <option value={5}>5</option>
              <option value={6}>6</option>
              <option value={8}>8</option>
              <option value={10}>10</option>
              <option value={12}>12</option>
            </select>
          </div>
        )}
      </div>

      {viewType === "grid" ? showGrid() : showList()}

      {pages !== undefined && pages > 1 && (
        <div className="flex justify-center">
          <div className="flex">{pageButtons(page ?? 0, pages)}</div>
        </div>
      )}
    </div>
  );
}
