import useLogin from "./login";
import { useRequestBuilder } from "@snort/system-react";
import { EventKind, RequestBuilder } from "@snort/system";
import { appendDedupe, dedupe, removeUndefined, sanitizeRelayUrl } from "@snort/shared";
import { ServerUrl } from "../const";

const DefaultMediaServers = ["https://blossom.band/", "https://blossom.primal.net", ServerUrl];

export function useBlossomServers() {
  const login = useLogin();

  const rb = new RequestBuilder("media-servers");
  if (login?.pubkey) {
    rb.withFilter()
      .kinds([10_063 as EventKind])
      .authors([login.pubkey]);
  }
  const req = useRequestBuilder(rb);

  const servers = req === undefined ? undefined : dedupe(removeUndefined(req.flatMap((e) => e.tags.filter(t => t[0] === "server").map((t) => sanitizeRelayUrl(t[1])))));
  return appendDedupe(DefaultMediaServers, servers);
}