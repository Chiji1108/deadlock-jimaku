import { describe, expect, it } from "vitest";
import { app } from "./index";

describe("api", () => {
  it("is 404 for unknown paths", async () => {
    expect((await app.request("/v1/aliases/ja")).status).toBe(404);
  });
});
