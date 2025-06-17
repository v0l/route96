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
    <div className="bg-gray-800 border border-gray-700 rounded-lg p-4">
      <div className="flex items-center justify-between mb-2">
        <h4 className="font-medium text-blue-400">
          {fileName ? `Uploading ${fileName}` : "Uploading..."}
        </h4>
        <span className="text-sm text-gray-400">
          {percentage.toFixed(1)}%
        </span>
      </div>
      
      {/* Progress Bar */}
      <div className="w-full bg-gray-700 rounded-full h-2.5 mb-3">
        <div
          className="bg-blue-500 h-2.5 rounded-full transition-all duration-300"
          style={{ width: `${Math.min(100, percentage)}%` }}
        />
      </div>
      
      {/* Upload Stats */}
      <div className="grid grid-cols-1 md:grid-cols-3 gap-3 text-sm">
        <div>
          <span className="text-gray-400">Progress:</span>
          <span className="ml-2 font-medium">
            {FormatBytes(bytesUploaded)} / {FormatBytes(totalBytes)}
          </span>
        </div>
        
        <div>
          <span className="text-gray-400">Speed:</span>
          <span className="ml-2 font-medium text-green-400">
            {formatSpeed(averageSpeed)}
          </span>
        </div>
        
        <div>
          <span className="text-gray-400">ETA:</span>
          <span className="ml-2 font-medium">
            {formatTime(estimatedTimeRemaining)}
          </span>
        </div>
      </div>
    </div>
  );
}