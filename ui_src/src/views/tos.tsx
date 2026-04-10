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

interface LabelModel {
  name: string;
  model_type?: string;
}

interface LabelingPolicy {
  enabled: boolean;
  models: LabelModel[];
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

const retentionTooltips: Record<string, string> = {
  delete_unaccessed_days:
    "Files are deleted if they have no downloads for this many days. Files uploaded within the window get grace period.",
  delete_after_days:
    "Hard limit: all files older than this are deleted regardless of download activity.",
  delete_zero_egress_days:
    "Files that have NEVER been downloaded (egress_bytes = 0) are deleted after this many days, regardless of age.",
};

const mediaTooltips: Record<string, string> = {
  webp_conversion:
    "Images and videos are converted to WebP format for smaller file sizes and faster loading.",
  thumbnails:
    "Small WebP thumbnails are generated for images and videos.",
  identical_media_dedup:
    "Visually identical images are detected using perceptual hashing (pHash) to save storage.",
  reject_sensitive_exif:
    "Uploads containing GPS location, device info, or other EXIF metadata are rejected.",
  reject_steganography:
    "Uploads suspected of containing hidden data (high entropy, XMP manipulation, malformed MPF) are rejected.",
};

export default function Tos() {
  const location = useLocation();
  const [props, setProps] = useState<ServerProps | null>(null);
  const [loading, setLoading] = useState(true);
  const [hoveredTooltip, setHoveredTooltip] = useState<string | null>(null);

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

  const getModelDisplayName = (model: LabelModel): string => {
    return model.name || model.model_type || "Unknown";
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
              <div className="flex justify-between items-center group relative">
                <div className="flex items-center gap-2">
                  <span className="text-neutral-400">
                    Delete inactive files
                  </span>
                  <div
                    className="w-4 h-4 rounded-full bg-neutral-700 text-neutral-400 flex items-center justify-center text-xs cursor-help"
                    onMouseEnter={() => setHoveredTooltip("delete_unaccessed_days")}
                    onMouseLeave={() => setHoveredTooltip(null)}
                  >
                    ?
                  </div>
                </div>
                <span>
                  After {props.retention.delete_unaccessed_days} days without downloads
                </span>
                {hoveredTooltip === "delete_unaccessed_days" && (
                  <div className="absolute left-0 top-full mt-2 z-10 w-72 bg-neutral-800 border border-neutral-700 rounded-lg p-3 shadow-lg">
                    <p className="text-xs text-neutral-300">
                      {retentionTooltips.delete_unaccessed_days}
                    </p>
                  </div>
                )}
              </div>
            ) : (
              <div className="flex justify-between items-center group relative">
                <div className="flex items-center gap-2">
                  <span className="text-neutral-400">Inactive file deletion</span>
                  <div
                    className="w-4 h-4 rounded-full bg-neutral-700 text-neutral-400 flex items-center justify-center text-xs cursor-help"
                    onMouseEnter={() => setHoveredTooltip("delete_unaccessed_days")}
                    onMouseLeave={() => setHoveredTooltip(null)}
                  >
                    ?
                  </div>
                </div>
                <span>Disabled</span>
                {hoveredTooltip === "delete_unaccessed_days" && (
                  <div className="absolute left-0 top-full mt-2 z-10 w-72 bg-neutral-800 border border-neutral-700 rounded-lg p-3 shadow-lg">
                    <p className="text-xs text-neutral-300">
                      {retentionTooltips.delete_unaccessed_days}
                    </p>
                  </div>
                )}
              </div>
            )}

            {props.retention.delete_after_days ? (
              <div className="flex justify-between items-center group relative">
                <div className="flex items-center gap-2">
                  <span className="text-neutral-400">Maximum file age</span>
                  <div
                    className="w-4 h-4 rounded-full bg-neutral-700 text-neutral-400 flex items-center justify-center text-xs cursor-help"
                    onMouseEnter={() => setHoveredTooltip("delete_after_days")}
                    onMouseLeave={() => setHoveredTooltip(null)}
                  >
                    ?
                  </div>
                </div>
                <span>
                  {props.retention.delete_after_days} days (hard limit)
                </span>
                {hoveredTooltip === "delete_after_days" && (
                  <div className="absolute left-0 top-full mt-2 z-10 w-72 bg-neutral-800 border border-neutral-700 rounded-lg p-3 shadow-lg">
                    <p className="text-xs text-neutral-300">
                      {retentionTooltips.delete_after_days}
                    </p>
                  </div>
                )}
              </div>
            ) : (
              <div className="flex justify-between items-center group relative">
                <div className="flex items-center gap-2">
                  <span className="text-neutral-400">Maximum file age</span>
                  <div
                    className="w-4 h-4 rounded-full bg-neutral-700 text-neutral-400 flex items-center justify-center text-xs cursor-help"
                    onMouseEnter={() => setHoveredTooltip("delete_after_days")}
                    onMouseLeave={() => setHoveredTooltip(null)}
                  >
                    ?
                  </div>
                </div>
                <span>No limit</span>
                {hoveredTooltip === "delete_after_days" && (
                  <div className="absolute left-0 top-full mt-2 z-10 w-72 bg-neutral-800 border border-neutral-700 rounded-lg p-3 shadow-lg">
                    <p className="text-xs text-neutral-300">
                      {retentionTooltips.delete_after_days}
                    </p>
                  </div>
                )}
              </div>
            )}

            {props.retention.delete_zero_egress_days ? (
              <div className="flex justify-between items-center group relative">
                <div className="flex items-center gap-2">
                  <span className="text-neutral-400">
                    Delete never-downloaded files
                  </span>
                  <div
                    className="w-4 h-4 rounded-full bg-neutral-700 text-neutral-400 flex items-center justify-center text-xs cursor-help"
                    onMouseEnter={() => setHoveredTooltip("delete_zero_egress_days")}
                    onMouseLeave={() => setHoveredTooltip(null)}
                  >
                    ?
                  </div>
                </div>
                <span>
                  After {props.retention.delete_zero_egress_days} days
                </span>
                {hoveredTooltip === "delete_zero_egress_days" && (
                  <div className="absolute left-0 top-full mt-2 z-10 w-72 bg-neutral-800 border border-neutral-700 rounded-lg p-3 shadow-lg">
                    <p className="text-xs text-neutral-300">
                      {retentionTooltips.delete_zero_egress_days}
                    </p>
                  </div>
                )}
              </div>
            ) : (
              <div className="flex justify-between items-center group relative">
                <div className="flex items-center gap-2">
                  <span className="text-neutral-400">
                    Zero-egress file deletion
                  </span>
                  <div
                    className="w-4 h-4 rounded-full bg-neutral-700 text-neutral-400 flex items-center justify-center text-xs cursor-help"
                    onMouseEnter={() => setHoveredTooltip("delete_zero_egress_days")}
                    onMouseLeave={() => setHoveredTooltip(null)}
                  >
                    ?
                  </div>
                </div>
                <span>Disabled</span>
                {hoveredTooltip === "delete_zero_egress_days" && (
                  <div className="absolute left-0 top-full mt-2 z-10 w-72 bg-neutral-800 border border-neutral-700 rounded-lg p-3 shadow-lg">
                    <p className="text-xs text-neutral-300">
                      {retentionTooltips.delete_zero_egress_days}
                    </p>
                  </div>
                )}
              </div>
            )}
          </div>
        </section>

        {props.media_processing && (
          <section className="bg-neutral-900 rounded-lg p-6">
            <h2 className="text-lg font-semibold mb-4">Media Processing</h2>
            <div className="grid gap-3 text-sm">
              <div className="flex justify-between items-center group relative">
                <div className="flex items-center gap-2">
                  <span className="text-neutral-400">WebP conversion</span>
                  <div
                    className="w-4 h-4 rounded-full bg-neutral-700 text-neutral-400 flex items-center justify-center text-xs cursor-help"
                    onMouseEnter={() => setHoveredTooltip("webp_conversion")}
                    onMouseLeave={() => setHoveredTooltip(null)}
                  >
                    ?
                  </div>
                </div>
                <span className={props.media_processing.webp_conversion ? "text-green-400" : "text-neutral-500"}>
                  {props.media_processing.webp_conversion ? "Enabled" : "Disabled"}
                </span>
                {hoveredTooltip === "webp_conversion" && (
                  <div className="absolute left-0 top-full mt-2 z-10 w-72 bg-neutral-800 border border-neutral-700 rounded-lg p-3 shadow-lg">
                    <p className="text-xs text-neutral-300">
                      {mediaTooltips.webp_conversion}
                    </p>
                  </div>
                )}
              </div>
              <div className="flex justify-between items-center group relative">
                <div className="flex items-center gap-2">
                  <span className="text-neutral-400">Thumbnails</span>
                  <div
                    className="w-4 h-4 rounded-full bg-neutral-700 text-neutral-400 flex items-center justify-center text-xs cursor-help"
                    onMouseEnter={() => setHoveredTooltip("thumbnails")}
                    onMouseLeave={() => setHoveredTooltip(null)}
                  >
                    ?
                  </div>
                </div>
                <span className={props.media_processing.thumbnails ? "text-green-400" : "text-neutral-500"}>
                  {props.media_processing.thumbnails ? "Enabled" : "Disabled"}
                </span>
                {hoveredTooltip === "thumbnails" && (
                  <div className="absolute left-0 top-full mt-2 z-10 w-72 bg-neutral-800 border border-neutral-700 rounded-lg p-3 shadow-lg">
                    <p className="text-xs text-neutral-300">
                      {mediaTooltips.thumbnails}
                    </p>
                  </div>
                )}
              </div>
              <div className="flex justify-between items-center group relative">
                <div className="flex items-center gap-2">
                  <span className="text-neutral-400">Identical media dedup</span>
                  <div
                    className="w-4 h-4 rounded-full bg-neutral-700 text-neutral-400 flex items-center justify-center text-xs cursor-help"
                    onMouseEnter={() => setHoveredTooltip("identical_media_dedup")}
                    onMouseLeave={() => setHoveredTooltip(null)}
                  >
                    ?
                  </div>
                </div>
                <span className={props.media_processing.identical_media_dedup ? "text-green-400" : "text-neutral-500"}>
                  {props.media_processing.identical_media_dedup ? `Enabled (distance: ${props.media_processing.identical_media_dedup_distance})` : "Disabled"}
                </span>
                {hoveredTooltip === "identical_media_dedup" && (
                  <div className="absolute left-0 top-full mt-2 z-10 w-72 bg-neutral-800 border border-neutral-700 rounded-lg p-3 shadow-lg">
                    <p className="text-xs text-neutral-300">
                      {mediaTooltips.identical_media_dedup}
                    </p>
                  </div>
                )}
              </div>
              <div className="flex justify-between items-center group relative">
                <div className="flex items-center gap-2">
                  <span className="text-neutral-400">Reject sensitive EXIF</span>
                  <div
                    className="w-4 h-4 rounded-full bg-neutral-700 text-neutral-400 flex items-center justify-center text-xs cursor-help"
                    onMouseEnter={() => setHoveredTooltip("reject_sensitive_exif")}
                    onMouseLeave={() => setHoveredTooltip(null)}
                  >
                    ?
                  </div>
                </div>
                <span className={props.media_processing.reject_sensitive_exif ? "text-green-400" : "text-neutral-500"}>
                  {props.media_processing.reject_sensitive_exif ? "Enabled" : "Disabled"}
                </span>
                {hoveredTooltip === "reject_sensitive_exif" && (
                  <div className="absolute left-0 top-full mt-2 z-10 w-72 bg-neutral-800 border border-neutral-700 rounded-lg p-3 shadow-lg">
                    <p className="text-xs text-neutral-300">
                      {mediaTooltips.reject_sensitive_exif}
                    </p>
                  </div>
                )}
              </div>
              <div className="flex justify-between items-center group relative">
                <div className="flex items-center gap-2">
                  <span className="text-neutral-400">Reject steganography</span>
                  <div
                    className="w-4 h-4 rounded-full bg-neutral-700 text-neutral-400 flex items-center justify-center text-xs cursor-help"
                    onMouseEnter={() => setHoveredTooltip("reject_steganography")}
                    onMouseLeave={() => setHoveredTooltip(null)}
                  >
                    ?
                  </div>
                </div>
                <span className={props.media_processing.reject_steganography ? "text-green-400" : "text-neutral-500"}>
                  {props.media_processing.reject_steganography ? "Enabled" : "Disabled"}
                </span>
                {hoveredTooltip === "reject_steganography" && (
                  <div className="absolute left-0 top-full mt-2 z-10 w-72 bg-neutral-800 border border-neutral-700 rounded-lg p-3 shadow-lg">
                    <p className="text-xs text-neutral-300">
                      {mediaTooltips.reject_steganography}
                    </p>
                  </div>
                )}
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
                        key={model.name}
                        className="bg-neutral-800 px-2 py-1 rounded text-xs"
                      >
                        {getModelDisplayName(model)}
                      </span>
                    ))}
                  </div>
                </div>
              )}
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
                        key={model.name}
                        className="bg-neutral-800 px-2 py-1 rounded text-xs"
                      >
                        {getModelDisplayName(model)}
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
