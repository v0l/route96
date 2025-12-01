import { useMemo } from "react";
import useLogin from "./login";
import { useRequestBuilder } from "@snort/system-react";
import { EventKind, RequestBuilder } from "@snort/system";
import { dedupe, removeUndefined, sanitizeRelayUrl } from "@snort/shared";
import { ServerUrl } from "../const";

const DefaultMediaServers = [ServerUrl]

export function useBlossomServers() {
  const login = useLogin();

  const rb = new RequestBuilder("media-servers");
  if (login?.publicKey) {
    rb.withFilter()
      .kinds([10_063 as EventKind])
      .authors([login.publicKey]);
  }
  const req = useRequestBuilder(rb);

  const servers = req === undefined ? undefined :
    req
      .flatMap((e) => e.tags.filter(t => t[0] === "server")
        .map((t) => t[1]));
  
  return useMemo(() => {
    return dedupe(removeUndefined([...DefaultMediaServers, ...(servers ?? [])].map(sanitizeRelayUrl)));
  }, [servers]);
}