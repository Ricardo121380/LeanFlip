import { describe, expect, it } from "vitest";
import i18n from "@/i18n";
import { errorMessage, errorDetails } from "./api";

describe("actionable backend errors", () => {
  it("uses the stable error code for recovery guidance and retains original diagnostics", async () => {
    await i18n.changeLanguage("zh");
    const error = { code: "codex.oauth.workspace_mismatch", message: "Live workspace mismatch" };
    expect(errorMessage(error)).toContain("重新登录正确账号");
    expect(errorMessage(error)).not.toContain("Live workspace mismatch");
    expect(errorDetails(error)).toBe("Live workspace mismatch");
    expect(errorMessage({ code: "io", message: "Permission denied" })).toContain("目录权限");
    await i18n.changeLanguage("en");
    expect(errorMessage(error)).toContain("different workspace");
    await i18n.changeLanguage("zh");
  });
  it("preserves specific messages and remains compatible with older backends", () => {
    expect(errorMessage({ code: "provider.command_confirmation_required", message: "Review this package first" })).toBe("Review this package first");
    expect(errorMessage("legacy failure")).toBe("legacy failure");
    expect(errorMessage(new Error("local failure"))).toBe("local failure");
    expect(errorMessage(undefined)).toContain("操作未完成");
  });
});
