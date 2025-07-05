import { useEffect, useState } from "react";
import { useParams, Navigate } from "react-router-dom";
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

  // Check if current user is admin
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

  // Load user info
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
      `Are you sure you want to delete ALL files for this user?\n\nThis action cannot be undone and will permanently delete:\n- ${userInfo?.file_count || 0} files\n- ${FormatBytes(userInfo?.total_size || 0, 2)} of storage\n\nType "DELETE" to confirm.`
    );
    
    if (!confirmed) return;
    
    const confirmText = window.prompt(
      'Please type "DELETE" to confirm this destructive action:'
    );
    
    if (confirmText !== "DELETE") {
      alert("Confirmation text did not match. Operation cancelled.");
      return;
    }

    setPurging(true);
    setError(undefined);
    
    try {
      const r96 = new Route96(url, pub);
      await r96.purgeUser(pubkey);
      
      // Refresh user info to show updated counts
      const response = await r96.getUserInfo(pubkey, filesPage, 50);
      setUserInfo(response.data);
      
      alert("User account purged successfully. All files have been deleted.");
    } catch (e) {
      const message = e instanceof Error ? e.message : "Failed to purge user account";
      setError(message);
    } finally {
      setPurging(false);
    }
  }

  if (loading) {
    return (
      <div className="flex justify-center items-center h-64">
        <div className="text-lg text-neutral-400">Loading...</div>
      </div>
    );
  }

  if (!login) {
    return (
      <div className="max-w-md mx-auto bg-neutral-800 border border-neutral-700 rounded-lg shadow-sm">
        <div className="text-center p-6">
          <h2 className="text-xl font-semibold mb-4 text-neutral-100">Authentication Required</h2>
          <p className="text-neutral-300">
            Please log in to access the admin panel.
          </p>
        </div>
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
      <div className="space-y-8 px-4">
        <div className="flex items-center justify-between">
          <h1 className="text-3xl font-bold text-neutral-100">User Scope</h1>
          <a
            href="/admin"
            className="text-blue-400 hover:text-blue-300 underline"
          >
            ← Back to Admin
          </a>
        </div>
        <div className="bg-red-900 border border-red-700 text-red-200 px-4 py-3 rounded">
          {error}
        </div>
      </div>
    );
  }

  if (!userInfo) {
    return (
      <div className="flex justify-center items-center h-64">
        <div className="text-lg text-neutral-400">User not found</div>
      </div>
    );
  }

  return (
    <div className="space-y-8 px-4">
      <div className="flex items-center justify-between">
        <h1 className="text-3xl font-bold text-neutral-100">User Scope</h1>
        <a
          href="/admin"
          className="text-blue-400 hover:text-blue-300 underline"
        >
          ← Back to Admin
        </a>
      </div>

      {/* User Information Card */}
      <div className="bg-neutral-800 border border-neutral-700 rounded-lg shadow-sm">
        <div className="p-6">
          <h3 className="text-lg font-semibold mb-4 text-neutral-100">User Information</h3>
          <div className="grid gap-4 md:grid-cols-2">
            <div>
              <label className="block text-sm font-medium text-neutral-400">Public Key</label>
              <div className="text-neutral-100 font-mono text-sm break-all">
                {hexToBech32("npub", pubkey)}
              </div>
              <div className="text-neutral-400 font-mono text-xs break-all mt-1">
                {pubkey}
              </div>
            </div>
            <div>
              <label className="block text-sm font-medium text-neutral-400">Account Created</label>
              <div className="text-neutral-100">
                {new Date(userInfo.created).toLocaleDateString()}
              </div>
            </div>
            <div>
              <label className="block text-sm font-medium text-neutral-400">Files Uploaded</label>
              <div className="text-neutral-100">
                {userInfo.file_count} files
              </div>
            </div>
            <div>
              <label className="block text-sm font-medium text-neutral-400">Total Storage Used</label>
              <div className="text-neutral-100">
                {FormatBytes(userInfo.total_size, 2)}
              </div>
            </div>
            {userInfo.is_admin && (
              <div>
                <label className="block text-sm font-medium text-neutral-400">Role</label>
                <div className="text-yellow-400 font-semibold">Administrator</div>
              </div>
            )}
          </div>
          
          {/* Danger Zone */}
          <div className="mt-6 pt-6 border-t border-neutral-600">
            <h4 className="text-lg font-semibold mb-4 text-red-400">Danger Zone</h4>
            <div className="bg-red-900/20 border border-red-700/50 rounded-lg p-4">
              <div className="flex items-center justify-between">
                <div>
                  <h5 className="text-sm font-medium text-red-300">Purge User Account</h5>
                  <p className="text-sm text-red-400/80 mt-1">
                    Permanently delete all {userInfo.file_count} files for this user ({FormatBytes(userInfo.total_size, 2)})
                  </p>
                </div>
                <button
                  onClick={handlePurgeUser}
                  disabled={purging || userInfo.file_count === 0}
                  className="bg-red-600 hover:bg-red-500 disabled:bg-red-800 disabled:text-red-400 text-white px-4 py-2 rounded font-medium text-sm transition-colors"
                >
                  {purging ? "Purging..." : "Purge Account"}
                </button>
              </div>
            </div>
          </div>
        </div>
      </div>

      {/* Quota Information Card */}
      {(userInfo.quota !== undefined || userInfo.free_quota !== undefined) && (
        <div className="bg-neutral-800 border border-neutral-700 rounded-lg shadow-sm">
          <div className="p-6">
            <h3 className="text-lg font-semibold mb-4 text-neutral-100">Quota Information</h3>
            <div className="grid gap-4 md:grid-cols-3">
              {userInfo.free_quota !== undefined && (
                <div>
                  <label className="block text-sm font-medium text-neutral-400">Free Quota</label>
                  <div className="text-neutral-100">
                    {FormatBytes(userInfo.free_quota, 2)}
                  </div>
                </div>
              )}
              {userInfo.quota !== undefined && (
                <div>
                  <label className="block text-sm font-medium text-neutral-400">Paid Quota</label>
                  <div className="text-neutral-100">
                    {FormatBytes(userInfo.quota, 2)}
                  </div>
                </div>
              )}
              {userInfo.total_available_quota !== undefined && (
                <div>
                  <label className="block text-sm font-medium text-neutral-400">Total Available</label>
                  <div className="text-neutral-100">
                    {FormatBytes(userInfo.total_available_quota, 2)}
                  </div>
                </div>
              )}
              {userInfo.paid_until !== undefined && userInfo.paid_until > 0 && (
                <div className="md:col-span-3">
                  <label className="block text-sm font-medium text-neutral-400">Paid Until</label>
                  <div className="text-neutral-100">
                    {new Date(userInfo.paid_until * 1000).toLocaleDateString()}
                  </div>
                </div>
              )}
            </div>
          </div>
        </div>
      )}

      {/* Payment History Card */}
      {userInfo.payments && userInfo.payments.length > 0 && (
        <div className="bg-neutral-800 border border-neutral-700 rounded-lg shadow-sm">
          <div className="p-6">
            <h3 className="text-lg font-semibold mb-4 text-neutral-100">Payment History</h3>
            <div className="overflow-x-auto">
              <table className="min-w-full bg-neutral-800 border border-neutral-600 rounded-lg">
                <thead className="bg-neutral-700/50">
                  <tr>
                    <th className="px-4 py-3 text-left text-xs font-medium text-neutral-400 uppercase tracking-wider border-b border-neutral-600">
                      Date
                    </th>
                    <th className="px-4 py-3 text-left text-xs font-medium text-neutral-400 uppercase tracking-wider border-b border-neutral-600">
                      Amount
                    </th>
                    <th className="px-4 py-3 text-left text-xs font-medium text-neutral-400 uppercase tracking-wider border-b border-neutral-600">
                      Status
                    </th>
                  </tr>
                </thead>
                <tbody className="divide-y divide-neutral-600">
                  {userInfo.payments.map((payment: any, idx: number) => (
                    <tr key={idx} className="hover:bg-neutral-700/30">
                      <td className="px-4 py-3 text-sm text-neutral-100">
                        {new Date(payment.created).toLocaleDateString()}
                      </td>
                      <td className="px-4 py-3 text-sm text-neutral-100">
                        {payment.amount}
                      </td>
                      <td className="px-4 py-3 text-sm">
                        <span
                          className={`px-2 py-1 rounded text-xs ${
                            payment.is_paid
                              ? "bg-green-700 text-green-200"
                              : "bg-yellow-700 text-yellow-200"
                          }`}
                        >
                          {payment.is_paid ? "Paid" : "Pending"}
                        </span>
                      </td>
                    </tr>
                  ))}
                </tbody>
              </table>
            </div>
          </div>
        </div>
      )}

      {/* Files Card */}
      <div className="bg-neutral-800 border border-neutral-700 rounded-lg shadow-sm">
        <div className="p-6">
          <h3 className="text-lg font-semibold mb-4 text-neutral-100">
            User Files ({userInfo.files.total} total)
          </h3>
          {userInfo.files.files.length > 0 ? (
            <FileList
              files={userInfo.files.files}
              pages={Math.ceil(userInfo.files.total / userInfo.files.count)}
              page={userInfo.files.page}
              onPage={(x) => setFilesPage(x)}
            />
          ) : (
            <div className="text-neutral-400 text-center py-8">
              No files found for this user.
            </div>
          )}
        </div>
      </div>
    </div>
  );
}