import { describe, expect, it, vi, afterEach } from "vitest";
import { base64urlnopad } from "@scure/base";
import { Blossom, hashFromUrl } from "./blossom";
import {
  MirrorOp,
  groupFailures,
  runMirror,
  stoppedMessage,
} from "./mirror-run";

const HASH = "a".repeat(64);
vi.mock("@snort/shared", () => ({
  throwIfOffline: () => {},
  unixNow: () => 1_700_000_000,
}));
vi.mock("@snort/system", () => ({ EventKind: {}, EventPublisher: class {} }));
vi.mock("./progress", () => ({
  uploadWithProgress: async () => new Response(),
}));

const OTHER = "b".repeat(64);

function fakePublisher() {
  return {
    generic: async (fn: (eb: any) => any) => {
      const tags: string[][] = [];
      const eb = {
        kind: () => eb,
        tag: (t: string[]) => {
          tags.push(t);
          return eb;
        },
      };
      fn(eb);
      return { kind: 24242, tags };
    },
  } as any;
}

function tagsOf(auth: string): string[][] {
  const json = new TextDecoder().decode(
    base64urlnopad.decode(auth.replace("Nostr ", "")),
  );
  return JSON.parse(json).tags;
}

function installFakeServer() {
  const calls: string[][][] = [];
  vi.stubGlobal("window", { crypto: undefined });
  vi.stubGlobal(
    "fetch",
    vi.fn(async (_url: string, init: any) => {
      const tags = tagsOf(init.headers.authorization);
      calls.push(tags);
      const x = tags.find((t) => t[0] === "x");
      if (!x) {
        return new Response("", {
          status: 400,
          headers: {
            "X-Reason": "The fetched blob isn't the one the auth token names",
          },
        });
      }
      return new Response(JSON.stringify({ sha256: x[1], size: 1 }), {
        status: 200,
      });
    }),
  );
  return calls;
}

afterEach(() => vi.unstubAllGlobals());

describe("hashFromUrl", () => {
  it("reads the hash from the last path segment", () => {
    expect(hashFromUrl(`https://a.example/${HASH}`)).toBe(HASH);
    expect(hashFromUrl(`https://a.example/${HASH}.png`)).toBe(HASH);
  });
  it("rejects other urls", () => {
    expect(hashFromUrl("https://a.example/file.png")).toBeUndefined();
  });
});

describe("Blossom.mirror", () => {
  it("puts the x tag in the auth event", async () => {
    const calls = installFakeServer();
    const b = new Blossom("https://t.example", fakePublisher());
    await b.mirror("https://s.example/anything", HASH);
    expect(calls[0]).toContainEqual(["x", HASH]);
  });

  it("takes the hash from the url", async () => {
    const calls = installFakeServer();
    const b = new Blossom("https://t.example", fakePublisher());
    const rsp = await b.mirror(`https://s.example/${HASH}`);
    expect(calls[0]).toContainEqual(["x", HASH]);
    expect(rsp.sha256).toBe(HASH);
  });

  it("fails clearly with no hash anywhere", async () => {
    const calls = installFakeServer();
    const b = new Blossom("https://t.example", fakePublisher());
    await expect(b.mirror("https://s.example/file.png")).rejects.toThrow(
      /no sha256/,
    );
    expect(calls.length).toBe(0);
  });
});

describe("runMirror", () => {
  const ops = (server: string, n: number): MirrorOp[] =>
    Array.from({ length: n }, (_, i) => ({
      sha256: i.toString(16).padStart(64, "0"),
      url: "https://s.example/x",
      server,
    }));

  it("mirrors through a server that demands the x tag", async () => {
    installFakeServer();
    const b = new Blossom("https://t.example", fakePublisher());
    const out = await runMirror(
      [{ sha256: HASH, url: "u", server: "https://t.example" }],
      (op) => b.mirror(op.url, op.sha256),
    );
    expect(out.completed.length).toBe(1);
    expect(out.failed.length).toBe(0);
  });

  it("stops a server after 3 identical failures and carries on", async () => {
    const calls: string[] = [];
    const all = [
      ...ops("https://bad.example", 6),
      ...ops("https://good.example", 2),
    ];
    const out = await runMirror(all, async (op) => {
      calls.push(op.server);
      if (op.server.includes("bad")) throw new Error("nope");
    });
    expect(calls.filter((s) => s.includes("bad")).length).toBe(3);
    expect(out.completed.length).toBe(2);
    expect(out.stopped).toEqual(["https://bad.example"]);
    expect(out.failed.length).toBe(6);
    expect(out.failed[5].message).toBe(stoppedMessage("https://bad.example"));
    expect(stoppedMessage("https://bad.example")).toBe(
      "stopped for bad.example after 3 identical errors",
    );
  });

  it("does not stop when errors differ or a success intervenes", async () => {
    let i = 0;
    const out = await runMirror(ops("https://s.example", 6), async () => {
      i++;
      if (i % 3 !== 0) throw new Error("same");
    });
    expect(out.stopped).toEqual([]);
    expect(out.completed.length).toBe(2);
  });
});

describe("groupFailures", () => {
  it("groups identical errors with counts", () => {
    const op = (sha256: string): MirrorOp => ({
      sha256,
      url: "u",
      server: "https://t.example",
    });
    const groups = groupFailures([
      { op: op(HASH), message: "m" },
      { op: op(OTHER), message: "m" },
      { op: op(HASH), message: "m" },
      { op: op(HASH), message: "other" },
    ]);
    expect(groups.length).toBe(2);
    expect(groups[0]).toMatchObject({ count: 3, files: 2, host: "t.example" });
  });
});
