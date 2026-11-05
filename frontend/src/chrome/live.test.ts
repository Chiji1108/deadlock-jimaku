import { describe, expect, it } from "vitest";
import type { LiveText } from "../generated/LiveText";
import { Live } from "./live";
import type { Translator } from "./speech";

const hello: Translator = { translate: (text) => Promise.resolve(`en:${text}`) };
const settled = () => new Promise((resolve) => setTimeout(resolve));

describe("Live", () => {
  it("文をすぐ送り、訳せたらもう一度送る", async () => {
    const sent: LiveText[] = [];
    const live = new Live(new Map([["en", hello]]), (l) => sent.push(l));
    live.onText("さあ");
    await settled();
    expect(sent).toEqual([
      { text: "さあ", translations: {} },
      { text: "さあ", translations: { en: [{ t: "text", text: "en:さあ" }] } },
    ]);
  });

  it("区切ったら、前の文の訳を次の文に使わない", async () => {
    const sent: LiveText[] = [];
    const live = new Live(new Map([["en", hello]]), (l) => sent.push(l));
    live.onText("前の文");
    live.cut();
    await settled();
    live.onText("次");
    expect(sent.at(-1)).toEqual({ text: "次", translations: {} });
  });
});
