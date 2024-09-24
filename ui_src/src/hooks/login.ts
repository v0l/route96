import { useSyncExternalStore } from "react";
import { Login } from "../login";

export default function useLogin() {
  return useSyncExternalStore(
    (c) => Login.hook(c),
    () => Login.snapshot(),
  );
}
