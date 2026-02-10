import { useEffect, useState } from "react";
import { useParams, Navigate, Link } from "react-router-dom";
import { hexToBech32 } from "@snort/shared";
import { FormatBytes } from "../const";
import FileList from "./files";
import useLogin from "../hooks/login";
import usePublisher from "../hooks/publisher";
import { AdminSelf, AdminUserInfo, Route96 } from "../upload/admin";

export default function UserScope() {
  const { pubkey } = useParams<{ pubkey: string }>();
  const [self, setSelf] = useState<AdminSelf>();
  const [userInfo, setUserInfo] = useState<AdminUserInfo>();
  const [error, setError] = useState<string>();
  const [loading, setLoading] = useState(true);
  const [filesPage, setFilesPage] = useState(0);
  const [purging, setPurging] = useState(false);

  const login = useLogin();
  const pub = usePublisher();

  const url =
    import.meta.env.VITE_API_URL || `${location.protocol}//${location.host}`;

  useEffect(() => {
    if (pub && !self) {
      const r96 = new Route96(url, pub);
      r96
        .getSelf()
        .then((v) => {
          setSelf(v.data);
        })
        .catch(() => {
          setLoading(false);
        });
    }
  }, [pub, self, url]);

  useEffect(() => {
    if (pub && self?.is_admin && pubkey) {
      setLoading(true);
      const r96 = new Route96(url, pub);
      r96
        .getUserInfo(pubkey, filesPage, 50)
        .then((response) => {
          setUserInfo(response.data);
          setLoading(false);
        })
        .catch((e) => {
          setError(e.message || "Failed to load user information");
          setLoading(false);
        });
    }
  }, [pub, self?.is_admin, pubkey, filesPage, url]);

  async function handlePurgeUser() {
    if (!pub || !pubkey) return;
    
    const confirmed = window.confirm(
      `Delete ALL files for this user?\n\n${userInfo?.file_count || 0} files (${FormatBytes(userInfo?.total_size || 0, 2)})\n\nThis cannot be undone.`
    );
    
    if (!confirmed) return;
    
    const confirmText = window.prompt('Type "DELETE" to confirm:');
    
    if (confirmText !== "DELETE") {
      alert("Cancelled.");
      return;
    }

    setPurging(true);
    setError(undefined);
    
    try {
      const r96 = new Route96(url, pub);
      await r96.purgeUser(pubkey);
      
      const response = await r96.getUserInfo(pubkey, filesPage, 50);
      setUserInfo(response.data);
      
      alert("User purged.");
    } catch (e) {
      const message = e instanceof Error ? e.message : "Failed to purge user";
      setError(message);
    } finally {
      setPurging(false);
    }
  }

  if (loading) {
    return (
      <div className="flex justify-center items-center h-48">
        <div className="text-sm text-neutral-500">Loading...</div>
      </div>
    );
  }

  if (!login) {
    return (
      <div className="max-w-sm mx-auto bg-neutral-900 border border-neutral-800 rounded-sm p-4">
        <h2 className="text-sm font-medium mb-2 text-white">Authentication Required</h2>
        <p className="text-neutral-500 text-xs">
          Please log in to access the admin panel.
        </p>
      </div>
    );
  }

  if (!self?.is_admin) {
    return <Navigate to="/" replace />;
  }

  if (!pubkey) {
    return <Navigate to="/admin" replace />;
  }

  if (error) {
    return (
      <div className="space-y-4">
        <div className="flex items-center justify-between">
          <h1 className="text-xl font-medium text-white">User</h1>
          <Link to="/admin" className="text-xs text-neutral-500 hover:text-white">
            &larr; Back
          </Link>
        </div>
        <div className="bg-red-950 border border-red-900 text-red-200 px-3 py-2 rounded-sm text-sm">
          {error}
        </div>
      </div>
    );
  }

  if (!userInfo) {
    return (
      <div className="flex justify-center items-center h-48">
        <div className="text-sm text-neutral-500">User not found</div>
      </div>
    );
  }

  return (
    <div className="space-y-4">
      <div className="flex items-center justify-between">
        <h1 className="text-xl font-medium text-white">User</h1>
        <Link to="/admin" className="text-xs text-neutral-500 hover:text-white">
          &larr; Back
        </Link>
      </div>

      {/* User Info */}
      <div className="bg-neutral-900 border border-neutral-800 rounded-sm p-3">
        <h3 className="text-sm font-medium mb-3 text-white">Info</h3>
        <div className="grid gap-3 md:grid-cols-2 text-xs">
          <div>
            <label className="text-neutral-500">Public Key</label>
            <div className="text-neutral-300 font-mono break-all">
              {hexToBech32("npub", pubkey)}
            </div>
            <div className="text-neutral-600 font-mono text-xs break-all mt-0.5">
              {pubkey}
            </div>
          </div>
          <div>
            <label className="text-neutral-500">Created</label>
            <div className="text-neutral-300">
              {new Date(userInfo.created).toLocaleDateString()}
            </div>
          </div>
          <div>
            <label className="text-neutral-500">Files</label>
            <div className="text-neutral-300">{userInfo.file_count}</div>
          </div>
          <div>
            <label className="text-neutral-500">Storage</label>
            <div className="text-neutral-300">{FormatBytes(userInfo.total_size, 2)}</div>
          </div>
          {userInfo.is_admin && (
            <div>
              <label className="text-neutral-500">Role</label>
              <div className="text-yellow-400">Admin</div>
            </div>
          )}
        </div>
        
        {/* Danger Zone */}
        <div className="mt-4 pt-3 border-t border-neutral-800">
          <div className="flex items-center justify-between bg-red-950/30 border border-red-900/50 rounded-sm p-2">
            <div>
              <div className="text-xs text-red-400">Purge Account</div>
              <div className="text-xs text-red-500/70">
                Delete all {userInfo.file_count} files ({FormatBytes(userInfo.total_size, 2)})
              </div>
            </div>
            <button
              onClick={handlePurgeUser}
              disabled={purging || userInfo.file_count === 0}
              className="bg-red-600 hover:bg-red-500 disabled:bg-red-900 disabled:text-red-400 text-white px-2 py-1 rounded-sm text-xs"
            >
              {purging ? "..." : "Purge"}
            </button>
          </div>
        </div>
      </div>

      {/* Quota */}
      {(userInfo.quota !== undefined || userInfo.free_quota !== undefined) && (
        <div className="bg-neutral-900 border border-neutral-800 rounded-sm p-3">
          <h3 className="text-sm font-medium mb-3 text-white">Quota</h3>
          <div className="grid gap-2 md:grid-cols-3 text-xs">
            {userInfo.free_quota !== undefined && (
              <div>
                <label className="text-neutral-500">Free</label>
                <div className="text-neutral-300">{FormatBytes(userInfo.free_quota, 2)}</div>
              </div>
            )}
            {userInfo.quota !== undefined && (
              <div>
                <label className="text-neutral-500">Paid</label>
                <div className="text-neutral-300">{FormatBytes(userInfo.quota, 2)}</div>
              </div>
            )}
            {userInfo.total_available_quota !== undefined && (
              <div>
                <label className="text-neutral-500">Total</label>
                <div className="text-neutral-300">{FormatBytes(userInfo.total_available_quota, 2)}</div>
              </div>
            )}
            {userInfo.paid_until !== undefined && userInfo.paid_until > 0 && (
              <div className="md:col-span-3">
                <label className="text-neutral-500">Paid Until</label>
                <div className="text-neutral-300">
                  {new Date(userInfo.paid_until * 1000).toLocaleDateString()}
                </div>
              </div>
            )}
          </div>
        </div>
      )}

      {/* Payments */}
      {userInfo.payments && userInfo.payments.length > 0 && (
        <div className="bg-neutral-900 border border-neutral-800 rounded-sm p-3">
          <h3 className="text-sm font-medium mb-3 text-white">Payments</h3>
          <div className="overflow-x-auto">
            <table className="w-full text-xs bg-neutral-950 border border-neutral-800 rounded-sm">
              <thead>
                <tr className="border-b border-neutral-800">
                  <th className="px-2 py-1.5 text-left text-neutral-500">Date</th>
                  <th className="px-2 py-1.5 text-left text-neutral-500">Amount</th>
                  <th className="px-2 py-1.5 text-left text-neutral-500">Status</th>
                </tr>
              </thead>
              <tbody className="divide-y divide-neutral-800">
                {userInfo.payments.map((payment: any, idx: number) => (
                  <tr key={idx}>
                    <td className="px-2 py-1.5 text-neutral-300">
                      {new Date(payment.created).toLocaleDateString()}
                    </td>
                    <td className="px-2 py-1.5 text-neutral-300">{payment.amount}</td>
                    <td className="px-2 py-1.5">
                      <span className={`px-1 py-0.5 rounded-sm text-xs ${
                        payment.is_paid
                          ? "bg-green-950 text-green-400"
                          : "bg-yellow-950 text-yellow-400"
                      }`}>
                        {payment.is_paid ? "Paid" : "Pending"}
                      </span>
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        </div>
      )}

      {/* Files */}
      <div className="bg-neutral-900 border border-neutral-800 rounded-sm p-3">
        <h3 className="text-sm font-medium mb-3 text-white">
          Files ({userInfo.files.total})
        </h3>
        {userInfo.files.files.length > 0 ? (
          <FileList
            files={userInfo.files.files}
            pages={Math.ceil(userInfo.files.total / userInfo.files.count)}
            page={userInfo.files.page}
            onPage={(x) => setFilesPage(x)}
          />
        ) : (
          <div className="text-neutral-500 text-xs text-center py-4">
            No files
          </div>
        )}
      </div>
    </div>
  );
}
