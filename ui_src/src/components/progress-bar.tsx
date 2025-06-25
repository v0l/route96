import { UploadProgress, formatSpeed, formatTime } from "../upload/progress";
import { FormatBytes } from "../const";

interface ProgressBarProps {
  progress: UploadProgress;
  fileName?: string;
}

export default function ProgressBar({ progress, fileName }: ProgressBarProps) {
  const {
    percentage,
    bytesUploaded,
    totalBytes,
    averageSpeed,
    estimatedTimeRemaining,
  } = progress;

  return (
    <div className="bg-neutral-800 border border-neutral-700 rounded-lg shadow-sm">
      <div className="p-4 pb-2">
        <div className="flex items-center justify-between">
          <h4 className="font-medium text-neutral-100">
            {fileName ? `Uploading ${fileName}` : "Uploading..."}
          </h4>
          <span className="text-sm text-neutral-400">
            {percentage.toFixed(1)}%
          </span>
        </div>
      </div>
      <div className="p-4 space-y-4">
        {/* Progress Bar */}
        <div className="w-full bg-neutral-700 rounded-full h-2">
          <div 
            className="bg-neutral-300 h-2 rounded-full transition-all duration-300"
            style={{ width: `${Math.min(100, percentage)}%` }}
          />
        </div>
        
        {/* Upload Stats */}
        <div className="grid grid-cols-1 md:grid-cols-3 gap-3 text-sm">
          <div>
            <span className="text-neutral-400">Progress:</span>
            <span className="ml-2 font-medium text-neutral-200">
              {FormatBytes(bytesUploaded)} / {FormatBytes(totalBytes)}
            </span>
          </div>
          
          <div>
            <span className="text-neutral-400">Speed:</span>
            <span className="ml-2 font-medium text-green-400">
              {formatSpeed(averageSpeed)}
            </span>
          </div>
          
          <div>
            <span className="text-neutral-400">ETA:</span>
            <span className="ml-2 font-medium text-neutral-200">
              {formatTime(estimatedTimeRemaining)}
            </span>
          </div>
        </div>
      </div>
    </div>
  );
}