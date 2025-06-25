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
}: {
  files: Array<File | NostrEvent | FileInfo>;
  pages?: number;
  page?: number;
  onPage?: (n: number) => void;
  onDelete?: (id: string) => void;
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
    return <b className="text-neutral-400">No Files</b>;
  }

  function getGridClass() {
    const baseClasses = "grid gap-4";
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
    
    // Previous arrow
    ret.push(
      <button
        key="prev"
        onClick={() => onPage?.(page - 1)}
        disabled={page === 0}
        className="px-3 py-2 text-sm font-medium border border-neutral-600 bg-neutral-800 text-neutral-200 hover:bg-neutral-700 disabled:opacity-50 disabled:cursor-not-allowed rounded-l-md transition-colors"
      >
        ←
      </button>
    );

    let startPage = Math.max(0, page - Math.floor(maxVisiblePages / 2));
    let endPage = Math.min(totalPages - 1, startPage + maxVisiblePages - 1);
    
    // Adjust start if we're near the end
    if (endPage - startPage < maxVisiblePages - 1) {
      startPage = Math.max(0, endPage - maxVisiblePages + 1);
    }

    // First page + ellipsis if needed
    if (startPage > 0) {
      ret.push(
        <button
          key={0}
          onClick={() => onPage?.(0)}
          className="px-3 py-2 text-sm font-medium border-t border-b border-neutral-600 bg-neutral-800 text-neutral-200 hover:bg-neutral-700 transition-colors"
        >
          1
        </button>
      );
      if (startPage > 1) {
        ret.push(
          <span key="ellipsis1" className="px-3 py-2 text-sm text-neutral-400 border-t border-b border-neutral-600 bg-neutral-800">
            ...
          </span>
        );
      }
    }

    // Visible page numbers
    for (let x = startPage; x <= endPage; x++) {
      ret.push(
        <button
          key={x}
          onClick={() => onPage?.(x)}
          className={classNames(
            "px-3 py-2 text-sm font-medium border-t border-b border-neutral-600 transition-colors",
            {
              "bg-neutral-600 text-neutral-100 border-neutral-500": page === x,
              "bg-neutral-800 text-neutral-200 hover:bg-neutral-700": page !== x,
            },
          )}
        >
          {x + 1}
        </button>,
      );
    }

    // Last page + ellipsis if needed
    if (endPage < totalPages - 1) {
      if (endPage < totalPages - 2) {
        ret.push(
          <span key="ellipsis2" className="px-3 py-2 text-sm text-neutral-400 border-t border-b border-neutral-600 bg-neutral-800">
            ...
          </span>
        );
      }
      ret.push(
        <button
          key={totalPages - 1}
          onClick={() => onPage?.(totalPages - 1)}
          className="px-3 py-2 text-sm font-medium border-t border-b border-neutral-600 bg-neutral-800 text-neutral-200 hover:bg-neutral-700 transition-colors"
        >
          {totalPages}
        </button>
      );
    }

    // Next arrow
    ret.push(
      <button
        key="next"
        onClick={() => onPage?.(page + 1)}
        disabled={page === totalPages - 1}
        className="px-3 py-2 text-sm font-medium border border-neutral-600 bg-neutral-800 text-neutral-200 hover:bg-neutral-700 disabled:opacity-50 disabled:cursor-not-allowed rounded-r-md transition-colors"
      >
        →
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
              className="group relative rounded-lg aspect-square overflow-hidden bg-neutral-700 border border-neutral-600 hover:shadow-md transition-shadow"
            >
              <div className="absolute inset-0 flex flex-col items-center justify-center p-2 text-xs text-center opacity-0 group-hover:opacity-100 bg-black/75 text-white transition-opacity">
                <div className="font-medium mb-1">
                  {(info.name?.length ?? 0) === 0
                    ? "Untitled"
                    : info.name!.length > 20
                      ? `${info.name?.substring(0, 10)}...${info.name?.substring(info.name.length - 10)}`
                      : info.name}
                </div>
                <div className="text-neutral-200 mb-1">
                  {info.size && !isNaN(info.size)
                    ? FormatBytes(info.size, 2)
                    : ""}
                </div>
                <div className="text-neutral-200 mb-2">{info.type}</div>
                <div className="flex gap-2">
                  <a
                    href={info.url}
                    className="bg-neutral-700 hover:bg-neutral-600 text-white px-2 py-1 rounded text-xs"
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
                      className="bg-red-600 hover:bg-red-500 text-white px-2 py-1 rounded text-xs"
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
        <table className="min-w-full bg-neutral-800 border border-neutral-600 rounded-lg">
          <thead className="bg-neutral-700/50">
            <tr>
              <th className="px-4 py-3 text-left text-xs font-medium text-neutral-400 uppercase tracking-wider border-b border-neutral-600">
                Preview
              </th>
              <th className="px-4 py-3 text-left text-xs font-medium text-neutral-400 uppercase tracking-wider border-b border-neutral-600">
                Name
              </th>
              <th className="px-4 py-3 text-left text-xs font-medium text-neutral-400 uppercase tracking-wider border-b border-neutral-600">
                Type
              </th>
              <th className="px-4 py-3 text-left text-xs font-medium text-neutral-400 uppercase tracking-wider border-b border-neutral-600">
                Size
              </th>
              {files.some((i) => "uploader" in i) && (
                <th className="px-4 py-3 text-left text-xs font-medium text-neutral-400 uppercase tracking-wider border-b border-neutral-600">
                  Uploader
                </th>
              )}
              <th className="px-4 py-3 text-left text-xs font-medium text-neutral-400 uppercase tracking-wider border-b border-neutral-600">
                Actions
              </th>
            </tr>
          </thead>
          <tbody className="divide-y divide-neutral-600">
            {files.map((a) => {
              const info = getInfo(a);
              return (
                <tr key={info.id} className="hover:bg-neutral-700/30">
                  <td className="px-4 py-3 w-16">
                    <div className="w-12 h-12 bg-neutral-700 rounded overflow-hidden">
                      {renderInner(info)}
                    </div>
                  </td>
                  <td className="px-4 py-3 text-sm text-neutral-100 break-all max-w-xs">
                    {(info.name?.length ?? 0) === 0 ? "<Untitled>" : info.name}
                  </td>
                  <td className="px-4 py-3 text-sm text-neutral-400">
                    {info.type}
                  </td>
                  <td className="px-4 py-3 text-sm text-neutral-400">
                    {info.size && !isNaN(info.size)
                      ? FormatBytes(info.size, 2)
                      : ""}
                  </td>
                  {info.uploader && (
                    <td className="px-4 py-3">
                      {info.uploader.map((a, idx) => (
                        <Profile
                          key={idx}
                          link={NostrLink.publicKey(a)}
                          size={20}
                        />
                      ))}
                    </td>
                  )}
                  <td className="px-4 py-3">
                    <div className="flex gap-2">
                      <a
                        href={info.url}
                        className="bg-neutral-700 hover:bg-neutral-600 text-white px-3 py-1 rounded text-xs"
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
                          className="bg-red-600 hover:bg-red-500 text-white px-3 py-1 rounded text-xs"
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
      <div className="flex justify-between items-center flex-wrap gap-4">
        <div className="flex rounded-lg border border-neutral-600 overflow-hidden">
          <button
            onClick={() => setViewType("grid")}
            className={`px-4 py-2 text-sm font-medium transition-colors ${
              viewType === "grid"
                ? "bg-neutral-600 text-neutral-100"
                : "bg-neutral-800 text-neutral-200 hover:bg-neutral-700"
            }`}
          >
            Grid
          </button>
          <button
            onClick={() => setViewType("list")}
            className={`px-4 py-2 text-sm font-medium transition-colors border-l border-neutral-600 ${
              viewType === "list"
                ? "bg-neutral-600 text-neutral-100"
                : "bg-neutral-800 text-neutral-200 hover:bg-neutral-700"
            }`}
          >
            List
          </button>
        </div>
        
        {viewType === "grid" && (
          <div className="flex items-center gap-2">
            <label className="text-sm font-medium text-neutral-400">
              Columns:
            </label>
            <select
              value={gridCols}
              onChange={(e) => setGridCols(parseInt(e.target.value))}
              className="flex h-9 w-16 rounded-md border border-neutral-600 bg-neutral-700 text-neutral-100 px-2 py-1 text-sm shadow-sm transition-colors focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-neutral-500 disabled:cursor-not-allowed disabled:opacity-50"
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
          <div className="flex rounded-lg border border-neutral-600 overflow-hidden">
            {pageButtons(page ?? 0, pages)}
          </div>
        </div>
      )}
    </div>
  );
}
