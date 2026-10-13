import { afterEach, describe, expect, it } from "vitest";
import { applyTheme, resolvedTheme } from "./theme";

afterEach(() => applyTheme(null));

describe("applyTheme", () => {
  it("forces light or dark on the root and returns to the system theme", () => {
    const root = document.documentElement;
    applyTheme("dark");
    expect(root.classList.contains("dark")).toBe(true);
    expect(root.classList.contains("light")).toBe(false);
    expect(resolvedTheme()).toBe("dark");

    applyTheme("light");
    expect(root.classList.contains("light")).toBe(true);
    expect(root.classList.contains("dark")).toBe(false);
    expect(resolvedTheme()).toBe("light");

    applyTheme(null);
    expect(root.classList.contains("light") || root.classList.contains("dark")).toBe(false);
  });
});
