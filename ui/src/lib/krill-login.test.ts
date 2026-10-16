import script from "../../../crates/app/src/krill_login.js?raw";
import { runInNewContext } from "node:vm";
import { describe, expect, it, vi } from "vitest";

describe("Krill native login capture", () => {
  function read(origin: string, pathname: string, frame = false) {
    const window: {top?: unknown} = {};
    window.top = frame ? {} : window;
    const getItem = vi.fn().mockReturnValue("test-login-token");
    const value = runInNewContext(script, { window, location: { origin, pathname }, localStorage: { getItem } });
    return { value, getItem };
  }
  it("reads only after official sign-in, never during auth, terms acceptance or in another origin/frame", () => {
    for (const path of ["/app", "/app/account", "/admin"]) {
      const result = read("https://www.krill-code.com", path);
      expect(result.value).toBe("test-login-token");
      expect(result.getItem).toHaveBeenCalledExactlyOnceWith("krill_jwt");
    }
    for (const [origin, path, frame] of [
      ["https://www.krill-code.com", "/login", false],
      ["https://www.krill-code.com", "/accept-tos", false],
      ["https://www.krill-code.com", "/auth/callback", false],
      ["https://www.krill-code.com", "/application", false],
      ["https://www.krill-code.com", "/app", true],
      ["https://accounts.google.com", "/app", false],
      ["https://www.krill-code.com.evil.test", "/app", false],
    ] as const) {
      const result = read(origin, path, frame);
      expect(result.value).toBeNull();
      expect(result.getItem).not.toHaveBeenCalled();
    }
  });
});
