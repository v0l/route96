import { describe, expect, it, vi } from "vitest";
import { renderToStaticMarkup } from "react-dom/server";
import { EventPublisher } from "@snort/system";
import ConfigEditor from "./config-editor";

vi.mock("@snort/system", () => ({
  NostrLink: class {},
  tryParseNostrLink: () => undefined,
}));
vi.mock("../upload/admin", () => ({ Route96: class {} }));
vi.mock("../const", () => ({ FormatBytes: (n: number) => `${n} B` }));
vi.mock("../hooks/login", () => ({ default: () => undefined }));
vi.mock("./profile", () => ({ default: () => null }));

function render(config: { key: string; value: string }[]) {
  return renderToStaticMarkup(
    <ConfigEditor
      config={config}
      pub={{} as EventPublisher}
      url="https://example.com"
      onSave={async () => {}}
      onDelete={async () => {}}
    />,
  );
}

describe("ConfigEditor payment repair window", () => {
  it("shows a typed days control with its default", () => {
    const html = render([]);
    expect(html).toContain("Lightning payment repair window");
    expect(html).toContain("payment_repair_days");
    expect(html).toContain("Defaults to 7");
  });

  it("shows the stored override as the current value", () => {
    const html = render([{ key: "payment_repair_days", value: "14" }]);
    expect(html).toMatch(/value="14"/);
    expect(html.split(">payment_repair_days<").length - 1).toBe(1);
  });
});
