import { useCallback, useEffect, useState } from "react";
import { Link } from "react-router-dom";
import { NostrLink } from "@snort/system";
import Pager from "./pager";
import Profile from "./profile";
import { FormatBytes } from "../const";
import { Route96, type SortOrder, type UserSort, type UserSummary } from "../upload/admin";
import type { EventPublisher } from "@snort/system";

const PAGE_SIZE = 50;

const sorts: { id: UserSort; label: string }[] = [
  { id: "created", label: "Joined" },
  { id: "size", label: "Size" },
  { id: "uploads", label: "Uploads" },
  { id: "egress", label: "Egress" },
  { id: "last_upload", label: "Last upload" },
];

export default function Users({ pub, url }: { pub: EventPublisher; url: string }) {
  const [users, setUsers] = useState<UserSummary[]>([]);
  const [total, setTotal] = useState(0);
  const [page, setPage] = useState(0);
  const [sort, setSort] = useState<UserSort>("created");
  const [order, setOrder] = useState<SortOrder>("desc");
  const [banned, setBanned] = useState(false);
  const [withBalance, setWithBalance] = useState(false);
  const [error, setError] = useState<string>();

  const load = useCallback(async () => {
    try {
      const r = await new Route96(url, pub).listUsers(
        page,
        PAGE_SIZE,
        sort,
        order,
        banned,
        withBalance,
      );
      setUsers(r.users);
      setTotal(r.total);
      setError(undefined);
    } catch (e) {
      setError(e instanceof Error ? e.message : "Failed to list users");
    }
  }, [pub, url, page, sort, order, banned, withBalance]);

  useEffect(() => {
    load();
  }, [load]);

  function toggleSort(id: UserSort) {
    if (sort === id) {
      setOrder(order === "desc" ? "asc" : "desc");
    } else {
      setSort(id);
      setOrder("desc");
    }
    setPage(0);
  }

  function header(id: UserSort, label: string) {
    return (
      <th
        className="px-2 py-1.5 text-left text-xs font-medium uppercase border-b border-neutral-800 cursor-pointer text-neutral-500 hover:text-neutral-300"
        onClick={() => toggleSort(id)}
      >
        {label}
        {sort === id ? (order === "desc" ? " ▼" : " ▲") : ""}
      </th>
    );
  }

  const pages = Math.ceil(total / PAGE_SIZE);
  const plain =
    "px-2 py-1.5 text-left text-xs font-medium text-neutral-500 uppercase border-b border-neutral-800";

  return (
    <div className="space-y-3">
      {error && <div className="text-red-400 text-xs">{error}</div>}
      <div className="flex flex-wrap items-center gap-3 text-xs text-neutral-300">
        <select
          className="h-7 rounded-sm border border-neutral-800 bg-neutral-950 px-2"
          value={sort}
          onChange={(e) => {
            setSort(e.target.value as UserSort);
            setPage(0);
          }}
        >
          {sorts.map((s) => (
            <option key={s.id} value={s.id}>
              {s.label}
            </option>
          ))}
        </select>
        <select
          className="h-7 rounded-sm border border-neutral-800 bg-neutral-950 px-2"
          value={order}
          onChange={(e) => setOrder(e.target.value as SortOrder)}
        >
          <option value="desc">Descending</option>
          <option value="asc">Ascending</option>
        </select>
        <label className="flex items-center gap-1">
          <input
            type="checkbox"
            checked={banned}
            onChange={(e) => {
              setBanned(e.target.checked);
              setPage(0);
            }}
          />
          Banned only
        </label>
        <label className="flex items-center gap-1">
          <input
            type="checkbox"
            checked={withBalance}
            onChange={(e) => {
              setWithBalance(e.target.checked);
              setPage(0);
            }}
          />
          With balance
        </label>
        <span className="text-neutral-500">{total} users</span>
      </div>
      <div className="overflow-x-auto">
        <table className="w-full text-xs bg-neutral-900 border border-neutral-800 rounded-sm">
          <thead className="bg-neutral-950">
            <tr>
              <th className={plain}>User</th>
              {header("uploads", "Uploads")}
              {header("size", "Size")}
              {header("egress", "Egress")}
              {header("last_upload", "Last upload")}
              {header("created", "Joined")}
              <th className={plain}>Status</th>
            </tr>
          </thead>
          <tbody className="divide-y divide-neutral-800">
            {users.map((u) => (
              <tr key={u.pubkey} className="hover:bg-neutral-800/50">
                <td className="px-2 py-1.5">
                  <div className="flex items-center gap-2">
                    <Profile link={NostrLink.publicKey(u.pubkey)} size={16} />
                    <Link
                      to={`/admin/user/${u.pubkey}`}
                      className="bg-neutral-800 hover:bg-neutral-700 text-white px-2 py-0.5 rounded-sm"
                    >
                      Manage
                    </Link>
                  </div>
                </td>
                <td className="px-2 py-1.5 text-neutral-300">{u.file_count}</td>
                <td className="px-2 py-1.5 text-neutral-300">{FormatBytes(u.total_size, 2)}</td>
                <td className="px-2 py-1.5 text-neutral-300">{FormatBytes(u.egress_bytes, 2)}</td>
                <td className="px-2 py-1.5 text-neutral-500">
                  {u.last_upload ? new Date(u.last_upload).toLocaleString() : "-"}
                </td>
                <td className="px-2 py-1.5 text-neutral-500">
                  {new Date(u.created).toLocaleDateString()}
                </td>
                <td className="px-2 py-1.5 space-x-1">
                  {u.is_admin && <span className="text-blue-400">admin</span>}
                  {u.banned && <span className="text-red-400">banned</span>}
                  {u.paid_until && new Date(u.paid_until) > new Date() && (
                    <span className="text-green-400">
                      paid until {new Date(u.paid_until).toLocaleDateString()}
                    </span>
                  )}
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      </div>
      <Pager page={page} pages={pages} onPage={setPage} />
    </div>
  );
}
