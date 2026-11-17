import classNames from "classnames";

function pageButtons(
  page: number,
  totalPages: number,
  onPage?: (n: number) => void,
) {
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
    </button>,
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
      </button>,
    );
    if (startPage > 1) {
      ret.push(
        <span
          key="ellipsis1"
          className="px-2 py-1 text-xs text-neutral-600 border-t border-b border-neutral-800 bg-neutral-900"
        >
          ...
        </span>,
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
        <span
          key="ellipsis2"
          className="px-2 py-1 text-xs text-neutral-600 border-t border-b border-neutral-800 bg-neutral-900"
        >
          ...
        </span>,
      );
    }
    ret.push(
      <button
        key={totalPages - 1}
        onClick={() => onPage?.(totalPages - 1)}
        className="px-2 py-1 text-xs border-t border-b border-neutral-800 bg-neutral-900 text-neutral-300 hover:bg-neutral-800 transition-colors"
      >
        {totalPages}
      </button>,
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
    </button>,
  );

  return ret;
}

export default function Pager({
  page,
  pages,
  onPage,
}: {
  page: number;
  pages?: number;
  onPage?: (n: number) => void;
}) {
  if (pages === undefined || pages <= 1) return null;
  return (
    <div className="flex justify-center">
      <div className="flex">{pageButtons(page, pages, onPage)}</div>
    </div>
  );
}
