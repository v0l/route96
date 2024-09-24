import { ExternalStore } from "@snort/shared";

export interface LoginSession {
  type: "nip7" | "amber";
  pubkey: string;
}

class LoginStore extends ExternalStore<LoginSession | undefined> {
  #session?: LoginSession;

  login(session: LoginSession) {
    this.#session = session;
    this.notifyChange();
  }

  takeSnapshot(): LoginSession | undefined {
    return this.#session ? { ...this.#session } : undefined;
  }
}

export const Login = new LoginStore();
