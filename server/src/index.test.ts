import { describe, expect, it } from "vitest";
import { app } from "./index";

describe("aliases", () => {
  it("returns the bundled aliases", async () => {
    const res = await app.request("/obs/v1/aliases/ja");
    expect(res.status).toBe(200);
    expect(res.headers.get("Cache-Control")).toContain("max-age");
    const aliases = (await res.json()) as Record<string, string[]>;
    expect(Object.keys(aliases).length).toBeGreaterThan(100);
  });

  it("is 404 for a language without aliases", async () => {
    for (const lang of ["xx", "__proto__", "constructor"]) {
      expect((await app.request(`/obs/v1/aliases/${lang}`)).status).toBe(404);
    }
  });
});
