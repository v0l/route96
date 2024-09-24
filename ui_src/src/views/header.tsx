import { Nip7Signer, NostrLink } from "@snort/system";
import Button from "../components/button";
import Profile from "../components/profile";
import useLogin from "../hooks/login";
import { Login } from "../login";

export default function Header() {
  const login = useLogin();

  async function tryLogin() {
    try {
      const n7 = new Nip7Signer();
      const pubkey = await n7.getPubKey();
      Login.login({
        type: "nip7",
        pubkey,
      });
    } catch {
      //ignore
    }
  }
  return (
    <div className="flex justify-between items-center">
      <div className="text-xl font-bold">route96</div>
      {login ? (
        <Profile link={NostrLink.publicKey(login.pubkey)} />
      ) : (
        <Button onClick={tryLogin}>Login</Button>
      )}
    </div>
  );
}
