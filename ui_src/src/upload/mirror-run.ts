export interface MirrorOp {
  sha256: string;
  url: string;
  server: string;
}

export interface MirrorFailure {
  op: MirrorOp;
  message: string;
  skipped?: boolean;
}

export interface MirrorOutcome {
  completed: MirrorOp[];
  failed: MirrorFailure[];
  stopped: string[];
}

export interface ErrorGroup {
  host: string;
  message: string;
  count: number;
  files: number;
  failures: MirrorFailure[];
}

export const MAX_IDENTICAL_FAILURES = 3;

export function hostOf(server: string): string {
  try {
    return new URL(server).hostname;
  } catch {
    return server;
  }
}

export function stoppedMessage(server: string): string {
  return `stopped for ${hostOf(server)} after ${MAX_IDENTICAL_FAILURES} identical errors`;
}

export async function runMirror(
  ops: MirrorOp[],
  mirror: (op: MirrorOp) => Promise<unknown>,
  onProgress?: (outcome: MirrorOutcome) => void,
): Promise<MirrorOutcome> {
  const outcome: MirrorOutcome = { completed: [], failed: [], stopped: [] };
  const streaks = new Map<string, { message: string; count: number }>();

  for (const op of ops) {
    if (outcome.stopped.includes(op.server)) {
      outcome.failed.push({
        op,
        message: stoppedMessage(op.server),
        skipped: true,
      });
      onProgress?.(outcome);
      continue;
    }
    try {
      await mirror(op);
      outcome.completed.push(op);
      streaks.delete(op.server);
    } catch (e) {
      const message = e instanceof Error ? e.message : "Unknown error";
      outcome.failed.push({ op, message });
      const streak = streaks.get(op.server);
      const count = streak && streak.message === message ? streak.count + 1 : 1;
      streaks.set(op.server, { message, count });
      if (count >= MAX_IDENTICAL_FAILURES) {
        outcome.stopped.push(op.server);
      }
    }
    onProgress?.(outcome);
  }
  return outcome;
}

export function groupFailures(failures: MirrorFailure[]): ErrorGroup[] {
  const groups = new Map<string, ErrorGroup>();
  for (const failure of failures) {
    if (failure.skipped) continue;
    const host = hostOf(failure.op.server);
    const key = `${host}\n${failure.message}`;
    let group = groups.get(key);
    if (!group) {
      group = {
        host,
        message: failure.message,
        count: 0,
        files: 0,
        failures: [],
      };
      groups.set(key, group);
    }
    group.count++;
    group.failures.push(failure);
  }
  for (const group of groups.values()) {
    group.files = new Set(group.failures.map((f) => f.op.sha256)).size;
  }
  return [...groups.values()];
}
