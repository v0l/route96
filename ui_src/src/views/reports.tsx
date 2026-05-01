import { NostrLink } from "@snort/system";
import classNames from "classnames";
import Profile from "../components/profile";
import { GroupedReport } from "../upload/admin";

export default function ReportList({
  reports,
  pages,
  page,
  onPage,
  onAcknowledge,
  onDeleteFile,
  selectedReports,
  onToggleSelect,
  onSelectAll,
  onBulkAcknowledge,
  onDeleteReports,
}: {
  reports: Array<GroupedReport>;
  pages?: number;
  page?: number;
  onPage?: (n: number) => void;
  onAcknowledge?: (reportId: number) => void;
  onDeleteFile?: (fileId: string) => void;
  selectedReports?: Set<number>;
  onToggleSelect?: (reportId: number) => void;
  onSelectAll?: (selected: boolean) => void;
  onBulkAcknowledge?: () => void;
  onDeleteReports?: () => void;
}) {
  if (reports.length === 0) {
    return <span className="text-neutral-500 text-sm">No Reports</span>;
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
            "px-2 py-1 text-xs border border-neutral-800 transition-colors",
            {
              "rounded-l-sm": x === start,
              "rounded-r-sm": x + 1 === n,
              "bg-white text-black": page === x,
              "bg-neutral-900 text-neutral-300 hover:bg-neutral-800":
                page !== x,
            },
          )}
        >
          {x + 1}
        </button>,
      );
    }

    return ret;
  }

  function getReportReason(reason: string): string {
    return reason || "No reason provided";
  }

  function formatDate(dateString: string): string {
    return new Date(dateString).toLocaleString();
  }

  const allSelected = reports.length > 0 && reports.every((r) => selectedReports?.has(r.latest_report_id));

  return (
    <div className="space-y-3">
      {/* Bulk action toolbar */}
      <div className="flex items-center justify-between bg-neutral-900 p-2 rounded-sm border border-neutral-800">
        <div className="flex items-center gap-2">
          <input
            type="checkbox"
            checked={allSelected}
            onChange={(e) => onSelectAll?.(e.target.checked)}
            className="w-4 h-4 rounded bg-neutral-800 border-neutral-700"
          />
          <span className="text-xs text-neutral-500">
            {selectedReports?.size || 0} selected
          </span>
        </div>
        <div className="flex gap-1">
          {onBulkAcknowledge && (
            <button
              onClick={onBulkAcknowledge}
              disabled={!selectedReports || selectedReports.size === 0}
              className="bg-neutral-800 hover:bg-neutral-700 disabled:opacity-50 disabled:cursor-not-allowed text-white px-2 py-1 rounded-sm text-xs"
            >
              Ack Selected
            </button>
          )}
          {onDeleteReports && (
            <button
              onClick={onDeleteReports}
              disabled={!selectedReports || selectedReports.size === 0}
              className="bg-red-900 hover:bg-red-800 disabled:opacity-50 disabled:cursor-not-allowed text-white px-2 py-1 rounded-sm text-xs"
            >
              Del Reports
            </button>
          )}
        </div>
      </div>

      <div className="overflow-x-auto">
        <table className="w-full text-xs bg-neutral-900 border border-neutral-800 rounded-sm">
          <thead className="bg-neutral-950">
            <tr>
              <th className="px-2 py-1.5 text-left text-xs font-medium text-neutral-500 uppercase border-b border-neutral-800 w-8">
                <input
                  type="checkbox"
                  checked={allSelected}
                  onChange={(e) => onSelectAll?.(e.target.checked)}
                  className="w-4 h-4 rounded bg-neutral-800 border-neutral-700"
                />
              </th>
              <th className="px-2 py-1.5 text-left text-xs font-medium text-neutral-500 uppercase border-b border-neutral-800">
                File
              </th>
              <th className="px-2 py-1.5 text-left text-xs font-medium text-neutral-500 uppercase border-b border-neutral-800">
                Reports
              </th>
              <th className="px-2 py-1.5 text-left text-xs font-medium text-neutral-500 uppercase border-b border-neutral-800">
                Reporter
              </th>
              <th className="px-2 py-1.5 text-left text-xs font-medium text-neutral-500 uppercase border-b border-neutral-800">
                Reason
              </th>
              <th className="px-2 py-1.5 text-left text-xs font-medium text-neutral-500 uppercase border-b border-neutral-800">
                Created
              </th>
              <th className="px-2 py-1.5 text-left text-xs font-medium text-neutral-500 uppercase border-b border-neutral-800">
                Actions
              </th>
            </tr>
          </thead>
          <tbody className="divide-y divide-neutral-800">
            {reports.map((report) => {
              const reason = getReportReason(report.reason);
              const isSelected = selectedReports?.has(report.latest_report_id);

              return (
                <tr key={report.latest_report_id} className={classNames("hover:bg-neutral-800/50", { "bg-neutral-800": isSelected })}>
                  <td className="px-2 py-1.5">
                    <input
                      type="checkbox"
                      checked={isSelected || false}
                      onChange={() => onToggleSelect?.(report.latest_report_id)}
                      className="w-4 h-4 rounded bg-neutral-800 border-neutral-700"
                    />
                  </td>
                  <td className="px-2 py-1.5 font-mono text-neutral-500">
                    <a
                      href={`/${report.file_id}`}
                      target="_blank"
                      rel="noopener noreferrer"
                      className="text-neutral-400 hover:text-white truncate block max-w-32"
                    >
                      {report.file_id.substring(0, 8)}...
                    </a>
                  </td>
                  <td className="px-2 py-1.5 text-neutral-300">
                    <span className="bg-red-900 text-white px-2 py-0.5 rounded-full text-xs">
                      {report.report_count}
                    </span>
                  </td>
                  <td className="px-2 py-1.5">
                    {report.reporter_pubkey ? (
                      <Profile
                        link={NostrLink.publicKey(report.reporter_pubkey)}
                        size={16}
                      />
                    ) : (
                      <span className="text-neutral-500">Unknown</span>
                    )}
                  </td>
                  <td className="px-2 py-1.5 text-neutral-400 max-w-32 truncate">
                    {reason}
                  </td>
                  <td className="px-2 py-1.5 text-neutral-500">
                    {formatDate(report.created)}
                  </td>
                  <td className="px-2 py-1.5">
                    <div className="flex gap-1">
                      <button
                        onClick={() => onAcknowledge?.(report.latest_report_id)}
                        className="bg-neutral-800 hover:bg-neutral-700 text-white px-2 py-1 rounded-sm text-xs"
                      >
                        Ack
                      </button>
                      <button
                        onClick={() => onDeleteFile?.(report.file_id)}
                        className="bg-neutral-800 hover:bg-neutral-700 text-white px-2 py-1 rounded-sm text-xs"
                      >
                        Del File
                      </button>
                    </div>
                  </td>
                </tr>
              );
            })}
          </tbody>
        </table>
      </div>

      {pages !== undefined && pages > 1 && (
        <div className="flex justify-center">
          <div className="flex">{pageButtons(page ?? 0, pages)}</div>
        </div>
      )}
    </div>
  );
}
