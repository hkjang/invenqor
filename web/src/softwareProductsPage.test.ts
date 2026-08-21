import {describe, expect, it} from "vitest";
import {
  confidencePresentation,
  softwareRoleLabel,
  softwareTopBuckets,
  type SoftwareSummary,
} from "./softwareProductsPage";

describe("software intelligence presentation", () => {
  it("makes confidence actionable instead of displaying a raw decimal", () => {
    expect(confidencePresentation(0.97)).toMatchObject({
      percent: 97, label: "매우 높음", tone: "good",
    });
    expect(confidencePresentation(0.72)).toMatchObject({
      percent: 72, label: "검토 권장", tone: "warn",
    });
    expect(confidencePresentation(2).percent).toBe(100);
  });

  it("uses operational Korean role names while preserving unknown catalog roles", () => {
    expect(softwareRoleLabel("database")).toBe("데이터베이스");
    expect(softwareRoleLabel("reverse_proxy")).toBe("리버스 프록시");
    expect(softwareRoleLabel("observability")).toBe("관측·모니터링");
    expect(softwareRoleLabel("asset_management")).toBe("자산 관리");
    expect(softwareRoleLabel("web_browser")).toBe("웹 브라우저");
    expect(softwareRoleLabel("productivity")).toBe("업무 생산성");
    expect(softwareRoleLabel("collaboration")).toBe("협업·커뮤니케이션");
    expect(softwareRoleLabel("custom_role")).toBe("custom_role");
  });

  it("turns product aggregates into instance distribution buckets", () => {
    const summary = {
      mapped_processes: 11,
      top_products: [
        {product_key: "nginx", product_name: "NGINX", role: "web_proxy", vendor: "F5", instances: 8, hosts: 7, running: 7, versions: ["1.26"]},
        {product_key: "postgresql", product_name: "PostgreSQL", role: "database", vendor: "PGDG", instances: 3, hosts: 3, running: 3, versions: ["16"]},
      ],
    } as SoftwareSummary;
    expect(softwareTopBuckets(summary)).toEqual([
      {label: "NGINX", count: 8},
      {label: "PostgreSQL", count: 3},
    ]);
    expect(softwareTopBuckets({top_products: null} as unknown as SoftwareSummary)).toEqual([]);
  });
});
