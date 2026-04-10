import { useEffect, useState } from "react";
import { useLocation } from "react-router-dom";

interface RetentionPolicy {
  delete_unaccessed_days: number | null;
  delete_after_days: number | null;
  delete_zero_egress_days: number | null;
}

interface MediaProcessingPolicy {
  webp_conversion: boolean;
  thumbnails: boolean;
  identical_media_dedup: boolean;
  identical_media_dedup_distance: number;
  reject_sensitive_exif: boolean;
  reject_steganography: boolean;
}

interface LabelingPolicy {
  enabled: boolean;
  models: string[];
  flag_terms: string[];
}

interface PaymentPolicy {
  enabled: boolean;
  currency: string;
  intervals: string[];
}

interface ServerProps {
  max_upload_size: number;
  public_url: string;
  retention: RetentionPolicy;
  media_processing?: MediaProcessingPolicy;
  labeling?: LabelingPolicy;
  payments?: PaymentPolicy | null;
}

export default function Tos() {
  const location = useLocation();
  const [props, setProps] = useState<ServerProps | null>(null);
  const [loading, setLoading] = useState(true);

  useEffect(() => {
    const url =
      import.meta.env.VITE_API_URL ||
      `${window.location.protocol}//${window.location.host}`;

    fetch(`${url}/props`)
      .then((res) => res.json())
      .then((data) => {
        setProps(data);
        setLoading(false);
      })
      .catch(() => {
        setLoading(false);
      });
  }, [location.pathname]);

  const formatSize = (bytes: number) => {
    const gb = bytes / (1024 * 1024 * 1024);
    if (gb >= 1) return `${gb.toFixed(2)} GB`;
    const mb = bytes / (1024 * 1024);
    return `${mb.toFixed(2)} MB`;
  };

  if (loading) {
    return (
      <div className="flex items-center justify-center py-12">
        <div className="text-neutral-500">Loading...</div>
      </div>
    );
  }

  if (!props) {
    return (
      <div className="flex items-center justify-center py-12">
        <div className="text-neutral-500">Failed to load server properties</div>
      </div>
    );
  }

  return (
    <div className="max-w-3xl mx-auto">
      <h1 className="text-2xl font-bold mb-6">Terms of Service</h1>

      <div className="space-y-6">
        <section className="bg-neutral-900 rounded-lg p-6">
          <h2 className="text-lg font-semibold mb-4">Upload Limits</h2>
          <div className="grid gap-2 text-sm">
            <div className="flex justify-between">
              <span className="text-neutral-400">Maximum file size</span>
              <span>{formatSize(props.max_upload_size)}</span>
            </div>
          </div>
        </section>

        <section className="bg-neutral-900 rounded-lg p-6">
          <h2 className="text-lg font-semibold mb-4">File Retention Policy</h2>
          <div className="grid gap-3 text-sm">
            {props.retention.delete_unaccessed_days ? (
              <div className="flex justify-between">
                <span className="text-neutral-400">
                  Delete inactive files
                </span>
                <span>
                  After {props.retention.delete_unaccessed_days} days without downloads
                </span>
              </div>
            ) : (
              <div className="flex justify-between">
                <span className="text-neutral-400">Inactive file deletion</span>
                <span>Disabled</span>
              </div>
            )}

            {props.retention.delete_after_days ? (
              <div className="flex justify-between">
                <span className="text-neutral-400">Maximum file age</span>
                <span>
                  {props.retention.delete_after_days} days (hard limit)
                </span>
              </div>
            ) : (
              <div className="flex justify-between">
                <span className="text-neutral-400">Maximum file age</span>
                <span>No limit</span>
              </div>
            )}

            {props.retention.delete_zero_egress_days ? (
              <div className="flex justify-between">
                <span className="text-neutral-400">
                  Delete never-downloaded files
                </span>
                <span>
                  After {props.retention.delete_zero_egress_days} days
                </span>
              </div>
            ) : (
              <div className="flex justify-between">
                <span className="text-neutral-400">
                  Zero-egress file deletion
                </span>
                <span>Disabled</span>
              </div>
            )}
          </div>
        </section>

        {props.media_processing && (
          <section className="bg-neutral-900 rounded-lg p-6">
            <h2 className="text-lg font-semibold mb-4">Media Processing</h2>
            <div className="grid gap-3 text-sm">
              <div className="flex justify-between">
                <span className="text-neutral-400">WebP conversion</span>
                <span className={props.media_processing.webp_conversion ? "text-green-400" : "text-neutral-500"}>
                  {props.media_processing.webp_conversion ? "Enabled" : "Disabled"}
                </span>
              </div>
              <div className="flex justify-between">
                <span className="text-neutral-400">Thumbnails</span>
                <span className={props.media_processing.thumbnails ? "text-green-400" : "text-neutral-500"}>
                  {props.media_processing.thumbnails ? "Enabled" : "Disabled"}
                </span>
              </div>
              <div className="flex justify-between">
                <span className="text-neutral-400">Identical media deduplication</span>
                <span className={props.media_processing.identical_media_dedup ? "text-green-400" : "text-neutral-500"}>
                  {props.media_processing.identical_media_dedup ? `Enabled (distance: ${props.media_processing.identical_media_dedup_distance})` : "Disabled"}
                </span>
              </div>
              <div className="flex justify-between">
                <span className="text-neutral-400">Reject sensitive EXIF</span>
                <span className={props.media_processing.reject_sensitive_exif ? "text-green-400" : "text-neutral-500"}>
                  {props.media_processing.reject_sensitive_exif ? "Enabled" : "Disabled"}
                </span>
              </div>
              <div className="flex justify-between">
                <span className="text-neutral-400">Reject steganography</span>
                <span className={props.media_processing.reject_steganography ? "text-green-400" : "text-neutral-500"}>
                  {props.media_processing.reject_steganography ? "Enabled" : "Disabled"}
                </span>
              </div>
            </div>
          </section>
        )}

        {props.labeling && (
          <section className="bg-neutral-900 rounded-lg p-6">
            <h2 className="text-lg font-semibold mb-4">AI Labeling</h2>
            <div className="grid gap-3 text-sm">
              <div className="flex justify-between">
                <span className="text-neutral-400">Labeling enabled</span>
                <span className={props.labeling.enabled ? "text-green-400" : "text-neutral-500"}>
                  {props.labeling.enabled ? "Yes" : "No"}
                </span>
              </div>
              {props.labeling.models.length > 0 && (
                <div>
                  <span className="text-neutral-400">Models</span>
                  <div className="mt-1 flex flex-wrap gap-2">
                    {props.labeling.models.map((model) => (
                      <span
                        key={model}
                        className="bg-neutral-800 px-2 py-1 rounded text-xs"
                      >
                        {model}
                      </span>
                    ))}
                  </div>
                </div>
              )}
              {props.labeling.flag_terms.length > 0 && (
                <div>
                  <span className="text-neutral-400">Flag terms</span>
                  <div className="mt-1 flex flex-wrap gap-2">
                    {props.labeling.flag_terms.map((term) => (
                      <span
                        key={term}
                        className="bg-red-950 text-red-300 px-2 py-1 rounded text-xs"
                      >
                        {term}
                      </span>
                    ))}
                  </div>
                </div>
              )}
            </div>
          </section>
        )}

        {props.payments && (
          <section className="bg-neutral-900 rounded-lg p-6">
            <h2 className="text-lg font-semibold mb-4">Payment Plans</h2>
            <div className="grid gap-3 text-sm">
              <div className="flex justify-between">
                <span className="text-neutral-400">Currency</span>
                <span>{props.payments.currency}</span>
              </div>
              <div>
                <span className="text-neutral-400">Available intervals</span>
                <div className="mt-1 flex flex-wrap gap-2">
                  {props.payments.intervals.map((interval) => (
                    <span
                      key={interval}
                      className="bg-neutral-800 px-2 py-1 rounded text-xs capitalize"
                    >
                      {interval}
                    </span>
                  ))}
                </div>
              </div>
            </div>
          </section>
        )}

        <section className="bg-neutral-900 rounded-lg p-6">
          <h2 className="text-lg font-semibold mb-4">Important Notes</h2>
          <ul className="list-disc list-inside space-y-2 text-sm text-neutral-300">
            <li>
              Files may be automatically deleted based on the retention policy above.
              Do not use this service as your only backup.
            </li>
            <li>
              Uploaded content may be processed for AI labeling, thumbnail generation,
              and format conversion.
            </li>
            <li>
              The server administrator reserves the right to remove content that
              violates applicable laws or terms.
            </li>
            <li>
              No warranty is provided for data availability or durability.
            </li>
          </ul>
        </section>
      </div>
    </div>
  );
}
