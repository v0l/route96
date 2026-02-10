import { NostrLink } from "@snort/system";
import classNames from "classnames";
import Profile from "../components/profile";
import { Report } from "../upload/admin";

export default function ReportList({
  reports,
  pages,
  page,
  onPage,
  onAcknowledge,
  onDeleteFile,
}: {
  reports: Array<Report>;
  pages?: number;
  page?: number;
  onPage?: (n: number) => void;
  onAcknowledge?: (reportId: number) => void;
  onDeleteFile?: (fileId: string) => void;
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
              "bg-neutral-900 text-neutral-300 hover:bg-neutral-800": page !== x,
            },
          )}
        >
          {x + 1}
        </button>,
      );
    }

    return ret;
  }

  function getReporterPubkey(eventJson: string): string | null {
    try {
      const event = JSON.parse(eventJson);
      return event.pubkey;
    } catch {
      return null;
    }
  }

  function getReportReason(eventJson: string): string {
    try {
      const event = JSON.parse(eventJson);
      return event.content || "No reason provided";
    } catch {
      return "Invalid event data";
    }
  }

  function formatDate(dateString: string): string {
    return new Date(dateString).toLocaleString();
  }

  return (
    <div className="space-y-3">
      <div className="overflow-x-auto">
        <table className="w-full text-xs bg-neutral-900 border border-neutral-800 rounded-sm">
          <thead className="bg-neutral-950">
            <tr>
              <th className="px-2 py-1.5 text-left text-xs font-medium text-neutral-500 uppercase border-b border-neutral-800">
                ID
              </th>
              <th className="px-2 py-1.5 text-left text-xs font-medium text-neutral-500 uppercase border-b border-neutral-800">
                File
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
              const reporterPubkey = getReporterPubkey(report.event_json);
              const reason = getReportReason(report.event_json);

              return (
                <tr key={report.id} className="hover:bg-neutral-800/50">
                  <td className="px-2 py-1.5 text-neutral-300">
                    {report.id}
                  </td>
                  <td className="px-2 py-1.5 font-mono text-neutral-500">
                    {report.file_id.substring(0, 8)}...
                  </td>
                  <td className="px-2 py-1.5">
                    {reporterPubkey ? (
                      <Profile
                        link={NostrLink.publicKey(reporterPubkey)}
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
                        onClick={() => onAcknowledge?.(report.id)}
                        className="bg-neutral-800 hover:bg-neutral-700 text-white px-2 py-0.5 rounded-sm text-xs"
                      >
                        Ack
                      </button>
                      <button
                        onClick={() => onDeleteFile?.(report.file_id)}
                        className="bg-red-600 hover:bg-red-500 text-white px-2 py-0.5 rounded-sm text-xs"
                      >
                        Del
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
