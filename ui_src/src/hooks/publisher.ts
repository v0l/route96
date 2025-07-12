import useLogin from "./login";
import { Login } from "../login";

export default function usePublisher() {
  const login = useLogin();
  if (login?.publicKey) {
    const signer = Login.getSigner();
    return signer;
  }
}
