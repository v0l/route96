import { EventPublisher, Nip7Signer } from "@snort/system";
import { useMemo } from "react";
import useLogin from "./login";

export default function usePublisher() {
  const login = useLogin();

  return useMemo(() => {
    switch (login?.type) {
      case "nip7":
        return new EventPublisher(new Nip7Signer(), login.pubkey);
      default:
        return undefined;
    }
  }, [login?.type, login?.pubkey]);
}
