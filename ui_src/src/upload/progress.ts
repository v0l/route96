// Upload progress tracking types and utilities

export interface UploadProgress {
  percentage: number;
  bytesUploaded: number;
  totalBytes: number;
  averageSpeed: number; // bytes per second
  estimatedTimeRemaining: number; // seconds
  startTime: number;
}

export interface UploadProgressCallback {
  (progress: UploadProgress): void;
}

export class ProgressTracker {
  private startTime: number;
  private lastUpdateTime: number;
  private bytesUploaded: number = 0;
  private totalBytes: number;
  private speedSamples: number[] = [];
  private maxSamples = 10; // Keep last 10 speed samples for averaging

  constructor(totalBytes: number) {
    this.totalBytes = totalBytes;
    this.startTime = Date.now();
    this.lastUpdateTime = this.startTime;
  }

  update(bytesUploaded: number): UploadProgress {
    const now = Date.now();
    const timeDiff = now - this.lastUpdateTime;
    
    // Calculate instantaneous speed
    if (timeDiff > 0) {
      const bytesDiff = bytesUploaded - this.bytesUploaded;
      const instantSpeed = (bytesDiff / timeDiff) * 1000; // bytes per second
      
      // Keep a rolling average of speed samples
      this.speedSamples.push(instantSpeed);
      if (this.speedSamples.length > this.maxSamples) {
        this.speedSamples.shift();
      }
    }

    this.bytesUploaded = bytesUploaded;
    this.lastUpdateTime = now;

    // Calculate average speed
    const averageSpeed = this.speedSamples.length > 0 
      ? this.speedSamples.reduce((sum, speed) => sum + speed, 0) / this.speedSamples.length
      : 0;

    // Calculate estimated time remaining
    const remainingBytes = this.totalBytes - bytesUploaded;
    const estimatedTimeRemaining = averageSpeed > 0 ? remainingBytes / averageSpeed : 0;

    return {
      percentage: (bytesUploaded / this.totalBytes) * 100,
      bytesUploaded,
      totalBytes: this.totalBytes,
      averageSpeed,
      estimatedTimeRemaining,
      startTime: this.startTime,
    };
  }
}

// Utility function to format speed for display
export function formatSpeed(bytesPerSecond: number): string {
  if (bytesPerSecond === 0) return "0 B/s";
  
  const units = ["B/s", "KB/s", "MB/s", "GB/s"];
  let value = bytesPerSecond;
  let unitIndex = 0;
  
  while (value >= 1024 && unitIndex < units.length - 1) {
    value /= 1024;
    unitIndex++;
  }
  
  return `${value.toFixed(1)} ${units[unitIndex]}`;
}

// Utility function to format time for display
export function formatTime(seconds: number): string {
  if (seconds === 0 || !isFinite(seconds)) return "--";
  
  const minutes = Math.floor(seconds / 60);
  const remainingSeconds = Math.floor(seconds % 60);
  
  if (minutes > 0) {
    return `${minutes}m ${remainingSeconds}s`;
  } else {
    return `${remainingSeconds}s`;
  }
}

// XMLHttpRequest wrapper with progress tracking
export function uploadWithProgress(
  url: string,
  method: string,
  body: BodyInit | null,
  headers: Record<string, string>,
  onProgress?: UploadProgressCallback,
): Promise<Response> {
  return new Promise((resolve, reject) => {
    const xhr = new XMLHttpRequest();
    
    // Determine total size
    let totalSize = 0;
    if (body instanceof File) {
      totalSize = body.size;
    } else if (body instanceof FormData) {
      // For FormData, we need to estimate size
      const formData = body as FormData;
      for (const [, value] of formData.entries()) {
        if (value instanceof File) {
          totalSize += value.size;
        } else if (typeof value === 'string') {
          totalSize += new Blob([value]).size;
        }
      }
    }

    const tracker = new ProgressTracker(totalSize);

    // Set up progress tracking
    if (onProgress && totalSize > 0) {
      xhr.upload.addEventListener('progress', (event) => {
        if (event.lengthComputable) {
          const progress = tracker.update(event.loaded);
          onProgress(progress);
        }
      });
    }

    // Set up response handling
    xhr.addEventListener('load', () => {
      const response = new Response(xhr.response, {
        status: xhr.status,
        statusText: xhr.statusText,
        headers: parseHeaders(xhr.getAllResponseHeaders()),
      });
      resolve(response);
    });

    xhr.addEventListener('error', () => {
      reject(new Error('Network error'));
    });

    xhr.addEventListener('abort', () => {
      reject(new Error('Upload aborted'));
    });

    // Configure request
    xhr.open(method, url);
    
    // Set headers
    for (const [key, value] of Object.entries(headers)) {
      xhr.setRequestHeader(key, value);
    }

    // Send request
    xhr.send(body as XMLHttpRequestBodyInit | Document | null);
  });
}

// Helper function to parse response headers
function parseHeaders(headerString: string): Headers {
  const headers = new Headers();
  const lines = headerString.trim().split('\r\n');
  
  for (const line of lines) {
    const colonIndex = line.indexOf(':');
    if (colonIndex > 0) {
      const name = line.substring(0, colonIndex).trim();
      const value = line.substring(colonIndex + 1).trim();
      headers.append(name, value);
    }
  }
  
  return headers;
}