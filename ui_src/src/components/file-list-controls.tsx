import type { FileStatSort, SortOrder } from "../upload/admin";

interface FileListControlsProps {
  mimeFilter: string | undefined;
  onMimeFilter: (v: string | undefined) => void;
  labelFilter: string | undefined;
  onLabelFilter: (v: string | undefined) => void;
  sortBy: FileStatSort;
  onSortBy: (v: FileStatSort) => void;
  sortOrder: SortOrder;
  onSortOrder: (v: SortOrder) => void;
}

export default function FileListControls({
  mimeFilter,
  onMimeFilter,
  labelFilter,
  onLabelFilter,
  sortBy,
  onSortBy,
  sortOrder,
  onSortOrder,
}: FileListControlsProps) {
  return (
    <div className="flex flex-wrap items-center gap-2">
      <select
        className="h-7 rounded-sm border border-neutral-800 bg-neutral-950 px-2 text-xs text-neutral-300"
        value={mimeFilter || ""}
        onChange={(e) => onMimeFilter(e.target.value || undefined)}
      >
        <option value="">All types</option>
        <option value="image/webp">WebP</option>
        <option value="image/jpeg">JPEG</option>
        <option value="image/png">PNG</option>
        <option value="image/gif">GIF</option>
        <option value="video/mp4">MP4</option>
        <option value="video/mov">MOV</option>
      </select>
      <input
        type="text"
        placeholder="Filter by label..."
        className="h-7 rounded-sm border border-neutral-800 bg-neutral-950 px-2 text-xs text-neutral-300 placeholder-neutral-600"
        value={labelFilter || ""}
        onChange={(e) => onLabelFilter(e.target.value || undefined)}
      />
      <select
        className="h-7 rounded-sm border border-neutral-800 bg-neutral-950 px-2 text-xs text-neutral-300"
        value={sortBy}
        onChange={(e) => onSortBy(e.target.value as FileStatSort)}
      >
        <option value="created">Created</option>
        <option value="egress_bytes">Egress</option>
        <option value="last_accessed">Last Accessed</option>
      </select>
      <select
        className="h-7 rounded-sm border border-neutral-800 bg-neutral-950 px-2 text-xs text-neutral-300"
        value={sortOrder}
        onChange={(e) => onSortOrder(e.target.value as SortOrder)}
      >
        <option value="desc">Desc</option>
        <option value="asc">Asc</option>
      </select>
    </div>
  );
}
