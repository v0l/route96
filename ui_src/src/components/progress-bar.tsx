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
    <div className="bg-neutral-950 border border-neutral-800 rounded-sm p-2">
      <div className="flex items-center justify-between mb-2">
        <span className="text-xs text-neutral-300">
          {fileName ? `Uploading ${fileName}` : "Uploading..."}
        </span>
        <span className="text-xs text-neutral-500">
          {percentage.toFixed(0)}%
        </span>
      </div>
      
      <div className="w-full bg-neutral-800 rounded-sm h-1 mb-2">
        <div 
          className="bg-white h-1 rounded-sm transition-all"
          style={{ width: `${Math.min(100, percentage)}%` }}
        />
      </div>
      
      <div className="flex justify-between text-xs text-neutral-500">
        <span>{FormatBytes(bytesUploaded)} / {FormatBytes(totalBytes)}</span>
        <span className="text-green-400">{formatSpeed(averageSpeed)}</span>
        <span>{formatTime(estimatedTimeRemaining)}</span>
      </div>
    </div>
  );
}
