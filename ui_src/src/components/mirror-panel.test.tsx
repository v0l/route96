import { describe, expect, it, vi } from "vitest";
import { renderToStaticMarkup } from "react-dom/server";
import { MirrorOp, runMirror } from "../upload/mirror-run";
import { MirrorPanelView } from "./mirror-panel";

vi.mock("../const", () => ({
  FormatBytes: (n: number) => `${(n / 1e6).toFixed(1)} MB`,
}));

const REASON = "The fetched blob isn't the one the auth token names";

async function failedRun() {
  const ops: MirrorOp[] = Array.from({ length: 5 }, (_, i) => ({
    sha256: i.toString(16).padStart(64, "0"),
    url: "https://s.example/x",
    server: "https://very-long-mirror-hostname.example.com",
  }));
  return runMirror(ops, async () => {
    throw new Error(REASON);
  });
}

async function render() {
  const outcome = await failedRun();
  return renderToStaticMarkup(
    <MirrorPanelView
      totalFiles={5}
      totalMirrorOperations={5}
      totalSize={123456789}
      serverCoverage={[
        {
          url: "https://very-long-mirror-hostname.example.com",
          hostname: "very-long-mirror-hostname.example.com",
          coveragePercentage: 40,
        },
      ]}
      progress={{
        total: 5,
        completed: 0,
        failed: outcome.failed.length,
        outcome,
        finished: true,
      }}
      onMirrorAll={() => {}}
      onRetry={() => {}}
    />,
  );
}

describe("MirrorPanelView", () => {
  it("renders grouped errors, the stop line and retry", async () => {
    const html = await render();
    expect(html).toContain(
      `very-long-mirror-hostname.example.com: The fetched blob isn&#x27;t the one the auth token names (3x, 3 files)`,
    );
    expect(html).toContain(
      "stopped for very-long-mirror-hostname.example.com after 3 identical errors",
    );
    expect(html).toContain("Retry failed");
    expect(html).toContain("5 failed");
  });

  it("keeps long text from widening the panel on a phone", async () => {
    const html = await render();
    expect(html).toContain("min-w-0");
    expect(html).toMatch(/truncate min-w-0/);
    expect(html).toContain("break-words");
    expect(html).toContain("break-all");
    expect(html).not.toMatch(/\bw-\[\d+px\]|\bmin-w-\[/);
  });
});
