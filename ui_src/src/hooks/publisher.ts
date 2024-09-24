import { EventPublisher, Nip7Signer } from "@snort/system";
import useLogin from "./login";

export default function usePublisher() {
  const login = useLogin();
  switch (login?.type) {
    case "nip7":
      return new EventPublisher(new Nip7Signer(), login.pubkey);
  }
}
