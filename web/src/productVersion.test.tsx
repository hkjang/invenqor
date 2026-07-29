import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";
import {
  formatServerVersion,
  ProductVersion,
  type SystemInfo,
} from "./productVersion";

const systemInfo: SystemInfo = {
  database_mode: "POSTGRES_ACTIVE",
  server_version: "0.2.2",
  commit: "1234567890abcdef",
  build_time: "2026-07-29T02:00:00Z",
};

describe("ProductVersion", () => {
  it("normalizes the version prefix", () => {
    expect(formatServerVersion("0.2.2")).toBe("v0.2.2");
    expect(formatServerVersion("v0.2.2")).toBe("v0.2.2");
    expect(formatServerVersion()).toBe("확인 중");
  });

  it("renders the compact server version with build details", () => {
    const markup = renderToStaticMarkup(
      <ProductVersion info={systemInfo} compact />,
    );
    expect(markup).toContain("Server v0.2.2");
    expect(markup).toContain("Invenqor Server v0.2.2");
    expect(markup).toContain("Commit 1234567890ab");
    expect(markup).toContain("Build 2026-07-29T02:00:00Z");
  });

  it("renders a safe pending state before system info loads", () => {
    const markup = renderToStaticMarkup(<ProductVersion info={null} />);
    expect(markup).toContain("INVENQOR SERVER");
    expect(markup).toContain("확인 중");
    expect(markup).not.toContain("undefined");
  });
});
