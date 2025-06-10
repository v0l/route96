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
    return <b>No Reports</b>;
  }

  function pageButtons(page: number, n: number) {
    const ret = [];
    const start = 0;

    for (let x = start; x < n; x++) {
      ret.push(
        <div
          key={x}
          onClick={() => onPage?.(x)}
          className={classNames(
            "bg-neutral-700 hover:bg-neutral-600 min-w-8 text-center cursor-pointer font-bold",
            {
              "rounded-l-md": x === start,
              "rounded-r-md": x + 1 === n,
              "bg-neutral-400": page === x,
            },
          )}
        >
          {x + 1}
        </div>,
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
    <>
      <table className="w-full border-collapse border border-neutral-500">
        <thead>
          <tr className="bg-neutral-700">
            <th className="border border-neutral-500 py-2 px-4 text-left">Report ID</th>
            <th className="border border-neutral-500 py-2 px-4 text-left">File ID</th>
            <th className="border border-neutral-500 py-2 px-4 text-left">Reporter</th>
            <th className="border border-neutral-500 py-2 px-4 text-left">Reason</th>
            <th className="border border-neutral-500 py-2 px-4 text-left">Created</th>
            <th className="border border-neutral-500 py-2 px-4 text-left">Actions</th>
          </tr>
        </thead>
        <tbody>
          {reports.map((report) => {
            const reporterPubkey = getReporterPubkey(report.event_json);
            const reason = getReportReason(report.event_json);
            
            return (
              <tr key={report.id} className="hover:bg-neutral-700">
                <td className="border border-neutral-500 py-2 px-4">{report.id}</td>
                <td className="border border-neutral-500 py-2 px-4 font-mono text-sm">
                  {report.file_id.substring(0, 12)}...
                </td>
                <td className="border border-neutral-500 py-2 px-4">
                  {reporterPubkey ? (
                    <Profile link={NostrLink.publicKey(reporterPubkey)} size={20} />
                  ) : (
                    "Unknown"
                  )}
                </td>
                <td className="border border-neutral-500 py-2 px-4 max-w-xs truncate">
                  {reason}
                </td>
                <td className="border border-neutral-500 py-2 px-4">
                  {formatDate(report.created)}
                </td>
                <td className="border border-neutral-500 py-2 px-4">
                  <div className="flex gap-2">
                    <button
                      onClick={() => onAcknowledge?.(report.id)}
                      className="bg-blue-600 hover:bg-blue-700 px-2 py-1 rounded text-sm"
                    >
                      Acknowledge
                    </button>
                    <button
                      onClick={() => onDeleteFile?.(report.file_id)}
                      className="bg-red-600 hover:bg-red-700 px-2 py-1 rounded text-sm"
                    >
                      Delete File
                    </button>
                  </div>
                </td>
              </tr>
            );
          })}
        </tbody>
      </table>
      
      {pages !== undefined && (
        <>
          <div className="flex justify-center mt-4">
            <div className="flex gap-1">{pageButtons(page ?? 0, pages)}</div>
          </div>
        </>
      )}
    </>
  );
}