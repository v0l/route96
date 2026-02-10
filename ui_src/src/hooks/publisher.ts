import { EventPublisher } from "@snort/system";
import { useEffect, useState } from "react";
import useLogin from "./login";
import { Login } from "../login";

export default function usePublisher() {
  const login = useLogin();
  const [publisher, setPublisher] = useState<EventPublisher | undefined>();

  useEffect(() => {
    if (login?.publicKey) {
      // Use async signer initialization to wait for nip7 extension
      Login.getSignerAsync()
        .then(signer => {
          setPublisher(signer);
        })
        .catch(() => {
          setPublisher(undefined);
        });
    } else {
      setPublisher(undefined);
    }
  }, [login?.publicKey]);

  return publisher;
}
