import { beforeEach, describe, expect, it, vi } from "vitest";
import { render, screen } from "@testing-library/react";
import i18n from "@/i18n";
import type { Provider, QuotaSource, ToolId } from "@/lib/api";
import { TOOLS, toolMeta } from "@/lib/tools";
import { LightMap } from "./LightMap";
import type { Row } from "./model";

const card = (id: string, url: string, key: string): Provider => ({ id, name: id, settingsConfig: { env: { ANTHROPIC_BASE_URL: url, ANTHROPIC_AUTH_TOKEN: key } } });
const row = (providers: Provider[], current: string, tool = TOOLS[0]): Row => ({
  tool,
  state: "ready",
  active: providers.filter((p) => p.id === current),
  others: providers.filter((p) => p.id !== current),
  data: { tool: tool.id, kind: "switch", current, inLive: [], liveError: null, paths: [], providers },
});

const now = Date.parse("2026-10-16T08:00:00Z");
const source = (key: string, usedPercent: number, level: "ok" | "warn" | "out", users: [string, string][]): QuotaSource => ({
  key,
  kind: "relay",
  label: key,
  users: users.map(([tool, providerId]) => ({ tool: tool as ToolId, providerId, providerName: providerId, active: true })),
  snapshot: { windows: [{ minutes: 300, usedPercent, reset: false }], fetchedAt: now - 60_000, origin: "network", level, limitReached: false },
});

beforeEach(async () => {
  await i18n.changeLanguage("zh");
});

describe("LightMap quota marks", () => {
  it("marks each endpoint once per quota source", () => {
    const relay = card("relay", "https://relay.example/v1", "k1");
    const spare = card("spare", "https://relay.example/v1", "k2");
    const other = card("other", "https://other.example", "k3");
    const desktop = card("relay", "https://relay.example/v1", "k1");
    const shared = source("relay-k1", 85, "warn", [["claude", "relay"], ["claude-desktop", "relay"]]);
    const quota = new Map<string, QuotaSource>([
      ["claude:relay", shared],
      ["claude-desktop:relay", shared],
      ["claude:spare", source("relay-k2", 100, "out", [["claude", "spare"]])],
    ]);
    const rows = [row([relay, spare, other], "relay"), row([desktop], "relay", toolMeta("claude-desktop"))];
    render(<LightMap rows={rows} latency={{}} quota={quota} now={now} onSwitch={vi.fn()} onToggle={vi.fn()} />);

    const marks = document.querySelectorAll(".lm-hosts .qs");
    expect([...marks].map((mark) => [mark.textContent, mark.getAttribute("data-level")])).toEqual([
      ["5 小时 85%", "warn"],
      ["5 小时 100%", "out"],
    ]);
    expect(marks[0].getAttribute("title")).toContain("relay-k1");
    expect(screen.getByText("other.example").querySelector(".qs")).toBeNull();
  });

  it("shows no marks without quota data", () => {
    render(<LightMap rows={[row([card("relay", "https://relay.example/v1", "k1")], "relay")]} latency={{}} onSwitch={vi.fn()} onToggle={vi.fn()} />);
    expect(document.querySelector(".lm-hosts .qs")).toBeNull();
  });
});
